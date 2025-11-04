# Learnings Directory - DEPRECATED

**Status:** This directory is deprecated as of November 4, 2025.

---

## What Happened?

We migrated to the **[Diátaxis](https://diataxis.fr/) documentation framework**, which provides a clearer structure based on content purpose rather than origin.

---

## Where Did Content Go?

The "learnings" were distributed into appropriate Diátaxis categories:

### Gotchas and Patterns → How-To Guides
- `nix-flakes.md` → [`docs/how-to/nix-flakes.md`](../how-to/nix-flakes.md)
- Task-oriented solutions to common problems

### Technical Details → Reference
- `nostr-sdk.md` → [`docs/reference/nostr-sdk-upgrade.md`](../reference/nostr-sdk-upgrade.md) (planned)
- `git-http-backend.md` → [`docs/reference/git-protocol.md`](../reference/git-protocol.md)
- Factual technical information

### Concepts and Understanding → Explanation
- `grasp-audit.md` → Incorporated into [`docs/explanation/architecture.md`](../explanation/architecture.md)
- Discussion of design and architecture

---

## Why the Change?

The "learnings" category was ambiguous:
- Mixed gotchas, patterns, and concepts
- Unclear where to put new content
- Hard for readers to know what to expect

**Diátaxis provides clear categories:**
- **Tutorials** - Learning by doing
- **How-To** - Solving problems
- **Reference** - Looking up facts
- **Explanation** - Understanding concepts

See [`docs/README.md`](../README.md) for the new structure.

---

## For Content Authors

**Don't create new files here.** Instead, ask:

- "Can you teach me to...?" → [`docs/tutorials/`](../tutorials/)
- "How do I...?" → [`docs/how-to/`](../how-to/)
- "What is...?" → [`docs/reference/`](../reference/)
- "Why...?" → [`docs/explanation/`](../explanation/)

---

## Migration Status

- ✅ `nix-flakes.md` → Migrated to `how-to/nix-flakes.md`
- ⏳ `nostr-sdk.md` → Being incorporated into reference docs
- ✅ `grasp-audit.md` → Content in `explanation/architecture.md`

---

*This directory will be removed in a future cleanup.*  
*See [AGENTS.md](../../AGENTS.md) for documentation guidelines.*
