# relay.ngit.dev Migration Archive (January 2026)

**Status:** Reference only - not maintained

This directory contains the migration guide and scripts used during the 
relay.ngit.dev migration from ngit-relay to ngit-grasp in January 2026.

## ⚠️ Important

These materials are **archived for reference only**:

- **Scripts are specific to the relay.ngit.dev migration context**
- **Not designed for general use or other migrations**
- **May not work without modification**
- **Not maintained or supported**

Do not expect these scripts to work out of the box for your migration.

## What's Here

- `migration-guide.md` - Lessons learned, approach, and context from the actual migration
- `scripts/` - Analysis and validation scripts used during the migration process

## Why Archive This?

The relay.ngit.dev migration uncovered numerous bugs and edge cases that resulted 
in critical production fixes. See commits in the `4bc5-relay-ngit-dev-migration-v2` 
branch for details.

These materials document:

- Real-world migration challenges encountered
- Debugging approaches that worked in practice
- Context for production fixes merged from this branch
- Iterative script development during active migration

## Using This as Reference

If you're planning a migration to ngit-grasp:

1. **Read the migration guide** for conceptual approach and lessons learned
2. **Review the scripts** to understand what kinds of analysis were needed
3. **Expect to write your own scripts** tailored to your specific context
4. **Test extensively** in a non-production environment first

These materials show what was needed for one specific migration, not a 
general-purpose migration toolkit.

## Context

This migration was completed in January 2026 and resulted in relay.ngit.dev 
running ngit-grasp in production. The branch containing these materials also 
includes critical fixes for:

- Git protocol error handling
- Naughty list false positives
- Purgatory event tracking
- Sync startup issues
- Configuration management

Those fixes are now part of the main codebase.
