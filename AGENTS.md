# AI Agent Guidelines for ngit-grasp

**Purpose:** Ensure AI agents (and humans) maintain consistent documentation practices and avoid common pitfalls.

**Last Updated:** November 4, 2025

---

## 📁 Documentation Structure

### Overview

We use the **[Diátaxis](https://diataxis.fr/) framework** for all documentation. This prevents documentation sprawl by organizing content into four clear categories based on purpose and audience.

```
ngit-grasp/
├── README.md                    # Project overview (keep updated)
├── AGENTS.md                    # This file - agent guidelines
├── CHANGELOG.md                 # User-facing changes (semver)
│
├── work/                        # Temporary session files (.gitignore'd)
│   ├── README.md               # Only file committed to git
│   └── *.md                    # Session notes, status, plans (temporary)
│
├── docs/                        # All documentation (Diátaxis structure)
│   ├── README.md               # Navigation guide with quadrant diagram
│   │
│   ├── tutorials/              # Learning-oriented (practical + learning)
│   │   ├── getting-started.md  # First-time setup
│   │   └── first-audit.md      # Running your first audit
│   │
│   ├── how-to/                 # Task-oriented (practical + working)
│   │   ├── deploy.md           # Production deployment
│   │   ├── nix-flakes.md       # Nix environment setup
│   │   ├── test-compliance.md  # Running compliance tests
│   │   └── upgrade-nostr-sdk.md # SDK upgrade guide
│   │
│   ├── reference/              # Information-oriented (theoretical + working)
│   │   ├── git-protocol.md     # Git Smart HTTP protocol
│   │   ├── grasp-protocol.md   # GRASP specification
│   │   ├── configuration.md    # All config options
│   │   ├── test-strategy.md    # Testing reference
│   │   └── api.md              # Internal API docs
│   │
│   ├── explanation/            # Understanding-oriented (theoretical + learning)
│   │   ├── architecture.md     # System design overview
│   │   ├── inline-authorization.md # Why inline auth?
│   │   ├── comparison.md       # vs ngit-relay
│   │   └── decisions.md        # Design decisions
│   │
│   ├── archive/                # Historical session notes
│   │   └── YYYY-MM-DD-*.md     # Completed work
│   │
│   └── learnings/              # DEPRECATED - migrated to Diátaxis
│       └── README.md           # Migration notice
│
├── grasp-audit/                # Audit tool subproject
│   ├── README.md              # Main audit docs
│   └── docs/                  # Follows same Diátaxis structure
│       ├── tutorials/
│       ├── how-to/
│       ├── reference/
│       └── explanation/
│
└── .ai/                        # AI assistant context (ignored in git)
    └── history/               # Conversation history
```

### Diátaxis Framework

All documentation MUST fit into one of four categories:

**📚 Tutorials** (`docs/tutorials/`)
- **Purpose:** Learning-oriented, teach by doing
- **Audience:** Newcomers, beginners
- **Style:** Step-by-step lessons with guaranteed outcomes
- **Examples:** Getting Started, First Audit
- **Question:** "Can you teach me to...?"

**🔧 How-To Guides** (`docs/how-to/`)
- **Purpose:** Task-oriented, solve problems
- **Audience:** Users with basic knowledge
- **Style:** Practical recipes and solutions
- **Examples:** Deploy, Configure, Troubleshoot
- **Question:** "How do I...?"

**📖 Reference** (`docs/reference/`)
- **Purpose:** Information-oriented, technical facts
- **Audience:** Users looking up specific information
- **Style:** Dry, factual, comprehensive
- **Examples:** API docs, Config options, Protocols
- **Question:** "What is...?"

**💡 Explanation** (`docs/explanation/`)
- **Purpose:** Understanding-oriented, clarify concepts
- **Audience:** Users wanting deeper understanding
- **Style:** Discussion, context, alternatives
- **Examples:** Architecture, Design Decisions, Comparisons
- **Question:** "Why...?"

**See:** [Diátaxis documentation](https://diataxis.fr/) for detailed guidance.

### File Type Guidelines

**Markdown (.md):**
- Primary format for all documentation
- Easy to read in plain text and rendered
- Supports code blocks, links, tables
- Version control friendly

**Text (.txt):**
- Only for visual ASCII art summaries
- Must be archived after session (never permanent)
- Examples: status boxes, visual diagrams
- Archive to `docs/archive/YYYY-MM-DD-name.txt`

**Other formats:**
- Avoid unless absolutely necessary
- If needed, document in README.md why

---

## 📋 Document Lifecycle

### 1. Working Documents (work/ Directory)

**Purpose:** Session-specific temporary files  
**Location:** `work/` directory (.gitignore'd)  
**Lifecycle:** Created → Used → Archived or Deleted  
**Retention:** Archive valuable content, delete rest at session end

**Examples:**
- `work/session-notes.md` → Session notes and progress
- `work/status.md` → Current status report
- `work/migration-plan.md` → Planning document
- `work/visual-summary.txt` → ASCII art summaries

**Rules:**
- ✅ Create ALL session-specific docs in `work/`
- ✅ Use descriptive names (no date prefix needed)
- ✅ Archive valuable content to `docs/archive/YYYY-MM-DD-name.md`
- ✅ Delete obsolete files at session end
- ✅ Keep `work/` clean (empty except README.md when not in session)
- ❌ Don't commit `work/` contents to git (except README.md)
- ❌ Don't reference `work/` docs from permanent documentation
- ❌ Don't let `work/` accumulate files between sessions

**Why work/ instead of root:**
- Keeps root clean (only README.md, AGENTS.md, CHANGELOG.md)
- Clear separation: permanent vs. temporary
- Not committed to git (reduces noise)
- Easy to clean up (just `rm -rf work/*`)

### 2. Permanent Documentation (docs/)

**Purpose:** Long-term reference, architecture, guides  
**Location:** `docs/` (organized by Diátaxis category)  
**Lifecycle:** Created → Maintained → Updated  
**Retention:** Permanent (version controlled)

**Structure:**
- `docs/tutorials/` - Learning-oriented lessons
- `docs/how-to/` - Task-oriented guides
- `docs/reference/` - Information-oriented facts
- `docs/explanation/` - Understanding-oriented discussion

**Examples:**
- `docs/tutorials/getting-started.md` - First-time setup
- `docs/how-to/deploy.md` - Deployment guide
- `docs/reference/configuration.md` - Config options
- `docs/explanation/architecture.md` - System design

**Rules:**
- ✅ Categorize by Diátaxis framework (tutorial/how-to/reference/explanation)
- ✅ Keep updated as project evolves
- ✅ Use clear structure and headings
- ✅ Link between related docs
- ❌ Don't duplicate information (use links)
- ❌ Don't include session-specific details
- ❌ Don't put docs in wrong category (see Diátaxis guide)

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

### 4. Learnings (DEPRECATED)

**Status:** `docs/learnings/` is deprecated - content migrated to Diátaxis structure

**Migration:**
- Gotchas and patterns → `docs/how-to/`
- Technical details → `docs/reference/`
- Understanding concepts → `docs/explanation/`

**Examples:**
- `learnings/nix-flakes.md` → `how-to/nix-flakes.md`
- `learnings/nostr-sdk.md` → `reference/nostr-sdk-upgrade.md`
- `learnings/git-http-backend.md` → `reference/git-protocol.md`

**Rules:**
- ❌ Don't create new files in `docs/learnings/`
- ✅ Migrate existing content to appropriate Diátaxis category
- ✅ Add redirect notice in old location

---

## 🔄 Cleanup Process

### When to Clean Up

**Trigger:** End of session OR `work/` has >5 files  
**Frequency:** End of each session (mandatory)  
**Responsibility:** AI agents should proactively clean up before session end

### Cleanup Steps

1. **Review work/ Directory**
   ```bash
   # List all working docs
   ls -la work/
   ```

2. **Extract to Diátaxis Categories**
   - Review each doc in `work/`
   - Extract valuable content to appropriate category:
     - Gotchas/solutions → `docs/how-to/`
     - Technical facts → `docs/reference/`
     - Concepts/design → `docs/explanation/`
     - Lessons → `docs/tutorials/`

3. **Archive Important Session Docs**
   ```bash
   # Archive valuable session docs with date prefix
   mv work/migration-complete.md docs/archive/2025-11-04-migration-complete.md
   mv work/visual-summary.txt docs/archive/2025-11-04-visual-summary.txt
   ```

4. **Delete Temporary Files**
   ```bash
   # Delete obsolete working docs
   rm work/status.md
   rm work/notes.md
   
   # Or clean everything
   rm -rf work/*
   # (work/README.md is safe - in .gitignore exception)
   ```

5. **Verify Clean State**
   ```bash
   # Root should only have these:
   ls *.md
   # README.md
   # AGENTS.md
   # (CHANGELOG.md when created)
   
   # work/ should be empty (except README.md)
   ls work/
   # README.md
   ```

6. **Commit Changes**
   - Commit new permanent docs
   - Commit archived docs
   - Note: work/ contents not committed (gitignored)

### Example Cleanup

```bash
# Before cleanup (messy root!)
ls *.md
# README.md
# AGENTS.md
# CURRENT_STATUS.md
# DIATAXIS_MIGRATION.md
# SUMMARY.md
# SESSION_NOTES.md
# ... (many more)

# After cleanup (clean root!)
ls *.md
# README.md
# AGENTS.md

# Working files in work/ during session
ls work/
# README.md
# session-notes.md
# status.md

# After session cleanup
ls work/
# README.md
# (all session files archived or deleted)

# Archived
ls docs/archive/ | tail -5
# 2025-11-04-diataxis-migration.md
# 2025-11-04-diataxis-complete.md
# 2025-11-04-diataxis-migration-visual.txt
# 2025-11-04-session-summary.md
# ...

# Permanent docs in Diátaxis structure
ls docs/tutorials/
# getting-started.md
# first-audit.md

ls docs/how-to/
# nix-flakes.md
# deploy.md
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

## 📄 File Format Guidelines

### When to Use .txt Files

**Use .txt ONLY for:**
- ASCII art visual summaries
- Box diagrams with Unicode characters
- Terminal-style status displays

**Examples of appropriate .txt content:**
```
╔════════════════════════════════════════╗
║  STATUS: ✅ COMPLETE                   ║
╚════════════════════════════════════════╝
```

**Rules:**
- ✅ Create in root during session for visual impact
- ✅ Archive immediately after session ends
- ✅ Use descriptive names: `CLEANUP_VISUAL_SUMMARY.txt`
- ❌ Never keep .txt files in root long-term
- ❌ Don't use .txt for regular documentation
- ❌ Don't duplicate information (use .md instead)

**Lifecycle:**
```
Create .txt → Use in session → Archive immediately
```

### When to Use .md Files

**Use .md for ALL documentation:**
- Architecture docs
- Session summaries
- Status reports
- Learnings
- Planning documents
- API documentation
- User guides

**Why markdown is preferred:**
- Renders nicely on GitHub/GitLab
- Supports code blocks with syntax highlighting
- Easy to link between documents
- Better for long-form content
- Version control friendly

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
   - Session-specific? → `work/` (temporary, gitignored)
   - Teaching beginners? → `docs/tutorials/`
   - Solving a problem? → `docs/how-to/`
   - Technical reference? → `docs/reference/`
   - Explaining concepts? → `docs/explanation/`
   - Historical? → `docs/archive/`

4. **Ask the Diátaxis questions:**
   - "Can you teach me to...?" → Tutorial
   - "How do I...?" → How-To
   - "What is...?" → Reference
   - "Why...?" → Explanation

5. **Use descriptive names**
   - Working docs: `session-notes.md`, `status.md` (in `work/`)
   - Archived docs: `YYYY-MM-DD-description.md` (in `docs/archive/`)
   - Tutorials: `getting-started.md`, `first-audit.md`
   - How-To: `deploy.md`, `nix-flakes.md`
   - Reference: `configuration.md`, `api.md`
   - Explanation: `architecture.md`, `decisions.md`

6. **Choose correct file format**
   - Use `.md` for all documentation (default)
   - Use `.txt` ONLY for ASCII art visual summaries
   - Archive `.txt` files immediately after session

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

1. **Clean up work/ directory (MANDATORY)**
   - Archive valuable session docs to `docs/archive/YYYY-MM-DD-*.md`
   - Delete temporary status reports
   - Extract content to Diátaxis categories if needed
   - Verify `work/` is empty (except README.md)

2. **Create session summary (if valuable)**
   - Archive to `docs/archive/YYYY-MM-DD-session-summary.md`
   - Include: accomplishments, next steps, blockers

3. **Update permanent docs**
   - Sync README.md with reality
   - Update relevant docs/ files
   - Commit changes

4. **Verify clean state**
   ```bash
   ls *.md  # Should only show README.md, AGENTS.md
   ls work/  # Should only show README.md
   ```

### Cleanup Time (End of Session)

1. **Review work/ directory**
   ```bash
   ls -la work/
   ```

2. **Extract content to appropriate Diátaxis category:**
   - Gotchas/solutions → `docs/how-to/`
   - Technical facts → `docs/reference/`
   - Concepts/design → `docs/explanation/`
   - Lessons → `docs/tutorials/`

3. **Archive valuable session docs**
   ```bash
   mv work/important-notes.md docs/archive/2025-11-04-session-notes.md
   mv work/visual-summary.txt docs/archive/2025-11-04-visual-summary.txt
   ```

4. **Delete temporary files**
   ```bash
   rm work/status.md
   rm work/temp-notes.md
   ```

5. **Verify clean state**
   ```bash
   ls *.md  # Only README.md, AGENTS.md
   ls work/  # Only README.md
   ```

6. **Commit permanent changes**
   - Commit new/updated permanent docs
   - Commit archived docs
   - Note: work/ not committed (gitignored)

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
- [ ] ARCHIVED marker at top (for .md files)
- [ ] Learnings extracted first (for .md files)
- [ ] Not referenced in active docs

### For .txt Files

- [ ] Contains only ASCII art/visual summaries
- [ ] Created in root for session use
- [ ] Archived immediately after session
- [ ] Not used for regular documentation
- [ ] Descriptive filename with purpose clear

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
9. **Use .md by default** - Only use .txt for ASCII art
10. **Archive .txt immediately** - Don't let them linger

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
