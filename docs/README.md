# ngit-grasp Documentation

## Overview

This directory contains comprehensive documentation for the ngit-grasp project.

## Documents

### For Review
- **[../REVIEW_SUMMARY.md](../REVIEW_SUMMARY.md)** - Start here! Executive summary of the architecture investigation and recommendations

### Architecture & Design
- **[ARCHITECTURE.md](ARCHITECTURE.md)** - Detailed technical architecture, component design, data flows, and implementation details
- **[DECISION_SUMMARY.md](DECISION_SUMMARY.md)** - Why we chose inline authorization over Git hooks
- **[COMPARISON.md](COMPARISON.md)** - Side-by-side comparison with the reference implementation (ngit-relay)

### Technical References
- **[GIT_PROTOCOL.md](GIT_PROTOCOL.md)** - Git Smart HTTP protocol reference, pkt-line format, and parsing examples
- **[TEST_STRATEGY.md](TEST_STRATEGY.md)** - Comprehensive testing strategy including reusable GRASP compliance testing tool

### Project Files
- **[../README.md](../README.md)** - Project overview, quick start, and feature list
- **[../.env.example](../.env.example)** - Configuration template
- **[../LICENSE](../LICENSE)** - MIT License

## Reading Guide

### If you want to understand the architecture decision:
1. Read [REVIEW_SUMMARY.md](../REVIEW_SUMMARY.md) - Executive summary
2. Read [DECISION_SUMMARY.md](DECISION_SUMMARY.md) - Detailed rationale
3. Skim [COMPARISON.md](COMPARISON.md) - See how we differ from reference

### If you want to implement:
1. Read [ARCHITECTURE.md](ARCHITECTURE.md) - Component design and code structure
2. Read [TEST_STRATEGY.md](TEST_STRATEGY.md) - Testing approach and compliance tool
3. Read [GIT_PROTOCOL.md](GIT_PROTOCOL.md) - Git protocol details
4. Review code examples in ARCHITECTURE.md

### If you want to deploy:
1. Read [README.md](../README.md) - Quick start
2. Review [.env.example](../.env.example) - Configuration
3. See deployment section in [ARCHITECTURE.md](ARCHITECTURE.md)

### If you're comparing with ngit-relay:
1. Read [COMPARISON.md](COMPARISON.md) - Detailed comparison
2. See architecture diagrams in both COMPARISON.md and ARCHITECTURE.md

## Key Concepts

### Inline Authorization
The core architectural decision: we validate Git pushes **inside the HTTP handler** before spawning Git, rather than using Git's pre-receive hooks.

**Benefits:**
- Better error messages (HTTP responses vs. hook stderr)
- Simpler deployment (no hook management)
- Easier testing (pure Rust)
- Better performance (skip Git for invalid pushes)

### GRASP Protocol
Git Relays Authorized via Signed-Nostr Proofs - a protocol for hosting Git repositories with Nostr-based authorization.

**Key Points:**
- Repository announcements (NIP-34 kind 30317)
- State announcements (NIP-34 kind 30318)
- Multi-maintainer support via recursive maintainer sets
- Push validation against signed state events

### Technology Stack
- **actix-web**: HTTP server
- **git-http-backend**: Git protocol handling (Rust crate)
- **nostr-relay-builder**: Nostr relay infrastructure (rust-nostr)
- **tokio**: Async runtime

## Status

**ALPHA** - Architecture design complete, implementation not yet started.

## Contributing

See [../README.md](../README.md) for contribution guidelines.

## Questions?

Open an issue or discussion on the repository.
