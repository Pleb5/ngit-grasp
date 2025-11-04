# ngit-grasp

A [GRASP](https://gitworkshop.dev/danconwaydev.com/grasp) (Git Relays Authorized via Signed-Nostr Proofs) implementation in Rust.

## Overview

`ngit-grasp` is a Rust-based implementation of the GRASP protocol, which enables decentralized Git repository hosting with Nostr-based authorization. This implementation combines:

- **Git Smart HTTP Backend**: Serves Git repositories over HTTP
- **Nostr Relay**: Stores and validates repository announcements and state events
- **Integrated Authorization**: Validates Git pushes against Nostr state events without requiring external hooks

Unlike the reference implementation ([ngit-relay](https://gitworkshop.dev/npub15qydau2hjma6ngxkl2cyar74wzyjshvl65za5k5rl69264ar2exs5cyejr/ngit-relay)) which uses nginx + git-http-backend + pre-receive hooks + Khatru (Go), `ngit-grasp` provides a unified Rust service that handles both Git and Nostr protocols natively.

## Status

**ALPHA** - Under active development. API and architecture subject to change.

## Key Features

- **Pure Rust Implementation**: Single binary, no external dependencies beyond Git itself
- **Integrated Authorization**: Push validation happens inline during the Git receive-pack operation
- **GRASP-01 Compliant**: Core service requirements for Git hosting with Nostr authorization
- **Extensible Architecture**: Designed to support GRASP-02 (Proactive Sync) and GRASP-05 (Archive) extensions
- **Developer-Friendly**: Built with modern Rust async patterns using tokio and actix-web

## Architecture Highlights

The key architectural decision is **inline authorization** rather than Git hooks:

- The `git-http-backend` crate provides low-level access to the Git protocol
- We intercept the `git-receive-pack` operation before spawning the Git process
- Push validation happens by checking the Nostr relay for the latest state event
- Only matching pushes are forwarded to the actual Git repository

This approach provides:
- **Better error messages**: Direct HTTP responses vs. hook stderr
- **Simpler deployment**: No hook management or symlinks
- **Tighter integration**: Shared state between Git and Nostr components
- **Easier testing**: Pure Rust unit and integration tests

## GRASP Compliance

### GRASP-01 (Core Service Requirements)
- ✅ NIP-01 compliant Nostr relay at `/`
- ✅ Accepts NIP-34 repository announcements and state events
- ✅ Git Smart HTTP service at `/<npub>/<identifier>.git`
- ✅ Push validation against Nostr state events
- ✅ Multi-maintainer support via recursive maintainer sets
- ✅ Support for `refs/nostr/<event-id>` for PRs
- ✅ CORS support for web-based Git clients
- ✅ NIP-11 relay information document

### GRASP-02 (Proactive Sync) - Planned
- 🔄 Proactive event sync from listed relays
- 🔄 Proactive Git data sync from listed clone URLs
- 🔄 PR data fetching and serving

### GRASP-05 (Archive) - Planned
- 🔄 Accept repositories not listing this instance
- 🔄 Backup/mirror mode operation

## Technology Stack

- **Rust**: Core language
- **actix-web**: HTTP server framework
- **git-http-backend**: Git protocol handling
- **nostr-relay-builder**: Nostr relay infrastructure from rust-nostr
- **nostr-sdk**: Nostr event handling and validation
- **tokio**: Async runtime

## Quick Start

```bash
# Clone the repository
git clone https://gitworkshop.dev/ngit-grasp
cd ngit-grasp

# Build (using Nix for reproducible environment)
nix develop -c cargo build --release

# Configure
cp .env.example .env
# Edit .env with your settings

# Run
nix develop -c cargo run --release

# Run tests
nix develop -c cargo test --lib
```

**Don't have Nix?** See [Getting Started Tutorial](docs/tutorials/getting-started.md) for alternative setup methods.

## Configuration

Environment variables (see `.env.example`):

- `NGIT_DOMAIN`: Your domain (e.g., `gitnostr.com`)
- `NGIT_OWNER_NPUB`: Relay owner's npub
- `NGIT_RELAY_NAME`: Relay name for NIP-11
- `NGIT_RELAY_DESCRIPTION`: Relay description
- `NGIT_GIT_DATA_PATH`: Path to store Git repositories
- `NGIT_RELAY_DATA_PATH`: Path to store Nostr events
- `NGIT_BIND_ADDRESS`: Server bind address (default: `127.0.0.1:8080`)

## Documentation

We use the **[Diátaxis](https://diataxis.fr/)** framework for documentation:

- **[Tutorials](docs/tutorials/)** - Learn by doing (Getting Started, First Audit)
- **[How-To Guides](docs/how-to/)** - Solve specific problems (Deploy, Configure)
- **[Reference](docs/reference/)** - Look up technical details (Config, Protocols)
- **[Explanation](docs/explanation/)** - Understand concepts (Architecture, Decisions)

**Start here:** [Documentation Index](docs/README.md)

## Development

See [Architecture Overview](docs/explanation/architecture.md) for system design and [Test Strategy](docs/reference/test-strategy.md) for testing approach.

### Running Tests

We have two test suites:

**1. Main Project Tests (ngit-grasp)**

```bash
# Run unit tests (no external dependencies)
nix develop -c cargo test --lib

# Run all integration tests (automatic relay management)
nix develop -c cargo test --test nip01_compliance --test nip34_announcements

# Run NIP-01 compliance tests
nix develop -c cargo test --test nip01_compliance

# Run NIP-34 announcement tests
nix develop -c cargo test --test nip34_announcements

# With detailed output
nix develop -c cargo test --test nip01_compliance -- --nocapture

# Run specific test
nix develop -c cargo test --test nip01_compliance test_nip01_smoke
```

**Integration tests automatically:**
- Start a fresh relay instance
- Run compliance tests using grasp-audit library
- Clean up when done
- No manual relay management needed!

**2. GRASP Audit Tool (grasp-audit)**

The audit tool tests GRASP compliance of any relay (including ours or external ones).

```bash
# Enter grasp-audit directory
cd grasp-audit

# Run unit tests
nix develop -c cargo test

# Test against any relay (including external ones)
nix develop -c cargo run -- --url wss://relay.example.com

# Or test against any external relay:
nix develop -c cargo run -- --url wss://relay.example.com
```

### Development Commands

```bash
# Run with logging
RUST_LOG=debug nix develop -c cargo run

# Check code
nix develop -c cargo clippy
nix develop -c cargo fmt --check

# Generate test coverage (requires tarpaulin)
nix develop -c cargo tarpaulin --out Html
```

**Note:** Always use `nix develop` to ensure the correct build environment. See [docs/how-to/nix-flakes.md](docs/how-to/nix-flakes.md) for details.

## Project Structure

```
ngit-grasp/
├── src/
│   ├── main.rs              # Entry point, server setup
│   ├── git/
│   │   ├── mod.rs           # Git module
│   │   ├── handler.rs       # Git HTTP handlers
│   │   └── authorization.rs # Push validation logic
│   ├── nostr/
│   │   ├── mod.rs           # Nostr module
│   │   ├── relay.rs         # Relay setup and policies
│   │   └── events.rs        # Event handlers
│   ├── storage/
│   │   ├── mod.rs           # Storage abstraction
│   │   └── repository.rs    # Repository management
│   └── config.rs            # Configuration
├── docs/
│   └── ARCHITECTURE.md      # Detailed architecture
├── tests/
│   ├── integration/         # Integration tests
│   └── fixtures/            # Test data
└── README.md
```

## Comparison with ngit-relay

| Feature | ngit-relay (Go) | ngit-grasp (Rust) |
|---------|----------------|-------------------|
| Language | Go | Rust |
| Components | nginx + git-http-backend + hooks + Khatru | Single integrated binary |
| Authorization | Pre-receive Git hook | Inline during receive-pack |
| Deployment | Docker + supervisord | Single binary |
| Testing | Go tests + shell scripts | Rust unit + integration tests |
| Performance | Good | Excellent (zero-copy, async) |

## Contributing

Contributions welcome! Please:

1. Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
2. Open an issue to discuss major changes
3. Follow Rust conventions and run `cargo fmt` + `cargo clippy`
4. Add tests for new functionality

## License

MIT License - see [LICENSE](LICENSE) for details

## Related Projects

- [GRASP Protocol](https://gitworkshop.dev/danconwaydev.com/grasp) - Protocol specification
- [ngit-relay](https://gitworkshop.dev/npub15qydau2hjma6ngxkl2cyar74wzyjshvl65za5k5rl69264ar2exs5cyejr/ngit-relay) - Reference implementation in Go
- [ngit](https://gitworkshop.dev/ngit) - Nostr Git plugin for git CLI
- [NIP-34](https://nips.nostr.com/34) - Git Stuff (Nostr protocol)

## Acknowledgments

- Reference implementation by [@DanConwayDev](https://gitworkshop.dev/danconwaydev.com)
- [rust-nostr](https://github.com/rust-nostr/nostr) team for excellent Nostr libraries
- Git community for the Smart HTTP protocol
