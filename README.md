# ngit-grasp

A [GRASP](https://gitworkshop.dev/danconwaydev.com/grasp) (Git Relays Authorized via Signed-Nostr Proofs) implementation in Rust.

## Overview

`ngit-grasp` is a Rust-based implementation of the GRASP protocol, which enables decentralized Git repository hosting with Nostr-based authorization. This implementation combines:

- **Git Smart HTTP Backend**: Serves Git repositories over HTTP
- **Nostr Relay**: Stores and validates repository announcements and state events
- **Integrated Authorization**: Validates Git pushes against Nostr state events without requiring external hooks

Unlike the reference implementation ([ngit-relay](https://gitworkshop.dev/npub15qydau2hjma6ngxkl2cyar74wzyjshvl65za5k5rl69264ar2exs5cyejr/ngit-relay)) which uses nginx + git-http-backend + pre-receive hooks + Khatru (Go), `ngit-grasp` provides a unified Rust service that handles both Git and Nostr protocols natively.

## Status

## Key Features

- **Pure Rust Implementation**: Single binary, no external dependencies beyond Git itself
- **Integrated Authorization**: Push validation happens inline during the Git receive-pack operation
- **GRASP-01 Compliant**: Core service requirements for Git hosting with Nostr authorization
- **Extensible Architecture**: Designed to support GRASP-02 (Proactive Sync) and GRASP-05 (Archive) extensions
- **Developer-Friendly**: Built with modern Rust async patterns using tokio and actix-web

## Architecture Highlights

The key architectural decision is **inline authorization** rather than Git hooks:

- Vendored and customised `git-http-backend` crate provides low-level access to the Git protocol
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

## Roadmap

### Purgatory

State events / PR / PR Update events without git data should be accepted with msg: "won't be served until git data arrives" or "in puratory awaiting git data" and not served by the main relay.
When the git data arrives, they get released from puratory. If git data doesn't arrive within 1 day, the events get deleted.

This ensures the grasp serve only serves these events when it can provide the git data to support them.

Why this is useful:

1. owner submits updated state event but loses connectivity before sending the new git data. The relay causing ngit-cli to fail to clone and other clients to show a warning that the git servers state doesn't align with nostr as relays only serve the latest state event (as its addressable).
   a. clients could be made more resilient if they know older versions of the state event served by a grasp server relate to the state they are currently storing.
   b. if clients just start using grasp servers (instead of other relays) then they will always be able to find the git data related to the latest versin of the event servered by a grasp server

2. serving PR events where the git data isn't accessable isn't useful.

### GRASP-02 (Proactive Sync)

- rust-nostr client websocket connection to other grasp servers listening for our repo.
- negentropy catchup
- look for missing data (from state or PR / PR update) then try and fetch from other grasp servers. for efficency look for it from other repos (ie repos of other maintainers). Do this on new state event / PR / PR update evnet and on a timer for events we know we don't have the data for.

#### Proactive Sync +

. look for announcement events on other relays / grasp servers that list our service.
. look on read/write relays of repo / PR / Patch / Issue author to get related comments. pass through stricter anti SPAM mechanism?

### Data effiency

dedupe git data = shared object database or (GIT_ALTERNATE_OBJECT_DIRECTORIES or .git/objects/info/alternates)

### Monitoring

ngit-grasp exposes Prometheus metrics at `/metrics` for connection tracking, Git operations, and Nostr events.

**Configuration Options:**

| Option | CLI Flag | Environment Variable | Default |
|--------|----------|---------------------|---------|
| Metrics enabled | `--metrics-enabled` | `NGIT_METRICS_ENABLED` | `true` |
| Connection abuse threshold | `--metrics-connection-per-ip-abuse-threshold` | `NGIT_METRICS_CONNECTION_PER_IP_ABUSE_THRESHOLD` | `10` |
| Top N repos | `--metrics-top-n-repos` | `NGIT_METRICS_TOP_N_REPOS` | `10` |

**Key Metrics:**
- WebSocket connections (active, unique IPs, flagged abusers)
- Git operations (clone/fetch/push rates, bandwidth, authorization results)
- Nostr events (received, stored, rejected by kind)
- Top N repositories by bandwidth

**Privacy:** IP addresses are never exposed in metrics - only aggregate counts.

See [Monitoring Overview](docs/explanation/monitoring.md) and [Prometheus Setup Guide](docs/how-to/prometheus-setup.md) for deployment.

### Delete Events

Git data related to deleted Repositories should be archvied (and deleted after 90 days), also events related to ONLY this repository.

### Grasp Server Removed from Announcement Event

Unless GRASP-05, This should cause the git data and events related ONLY to this repository to be archived (and deleted after 90 days).

### Mitigate DoS attack vector

Grasp servers can be DoS by pushing large amounts of git data to `refs/nostr/<event-id>` without having to first submit a signed nostr event. operators must temporarily disable pushes to `refs/nostr/*` without having recieved a signed event. This breaks the flow of sending PR / Update events in NIP-34 as the client doesnt know if the grasp server will accept git data / event so might include it as a server hint in `clone` without knowing whether the server will accept the data. Could an ephemeral event be sent to authorise or is that too complicated? Maybe require NIP-42 auth and authorise that IP address for the push based on WoT?

### Reject Commits with Secrets

This a useful feature of other git servers.

### Store all user grasp lists (for better grasp discovery)

✅ **Implemented**: Kind 10317 (User Grasp List) events are now accepted and synced from all relays for better GRASP repository discovery.

**Future enhancement**: We aspire to also accept and weekly sync kind 10002 (user relay lists) and kind 0 (user metadata) events, but only for authors of accepted events. This would require an additional state-driven layer 2 filter (see Roadmap in GRASP-02 Proactive Sync documentation).

**Future enhancement**: should we periodically scan relays in UserGraspLists to check for announcements that list our relay?

## Technology Stack

- **Rust**: Core language
- **actix-web**: HTTP server framework
- **git-http-backend**: Git protocol handling but vendored and customised for authorisation logic
- **nostr-relay-builder**: Nostr relay infrastructure from rust-nostr
- **nostr-sdk**: Nostr event handling and validation
- **tokio**: Async runtime

## Quick Start

```bash
# install ngit
curl -Ls https://ngit.dev/install.sh | bash
# Clone the repository
git clone nostr://danconwaydev.com/relay.ngit.dev/ngit-grasp
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

Configuration is loaded with the following priority (highest to lowest):

1. **CLI flags** (e.g., `--domain example.com`)
2. **Environment variables** (e.g., `NGIT_DOMAIN=example.com`)
3. **.env file** (loaded automatically if present)
4. **Built-in defaults**

This means CLI flags always take precedence over environment variables, which take precedence over `.env` file values.

### CLI Usage

```bash
# View all options with defaults
ngit-grasp --help

# Run with CLI flags (override everything else)
ngit-grasp --domain relay.example.com --owner-npub npub1... --bind-address 0.0.0.0:8080

# Mix CLI flags with environment variables
NGIT_OWNER_NPUB=npub1... ngit-grasp --domain relay.example.com
```

### Configuration Options

| Option            | CLI Flag              | Environment Variable     | Default                                      |
| ----------------- | --------------------- | ------------------------ | -------------------------------------------- |
| Domain            | `--domain`            | `NGIT_DOMAIN`            | (required)                                   |
| Owner npub        | `--owner-npub`        | `NGIT_OWNER_NPUB`        | (optional)                                   |
| Relay name        | `--relay-name`        | `NGIT_RELAY_NAME`        | `${domain} grasp relay`                      |
| Relay description | `--relay-description` | `NGIT_RELAY_DESCRIPTION` | `Git Nostr Relay - a grasp implementation`   |
| Git data path     | `--git-data-path`     | `NGIT_GIT_DATA_PATH`     | `./data/git` (temp dir for memory backend)   |
| Relay data path   | `--relay-data-path`   | `NGIT_RELAY_DATA_PATH`   | `./data/relay` (temp dir for memory backend) |
| Bind address      | `--bind-address`      | `NGIT_BIND_ADDRESS`      | `127.0.0.1:8080`                             |
| Database backend  | `--database-backend`  | `NGIT_DATABASE_BACKEND`  | `lmdb`                                       |

### Database Backends

- `lmdb`: LMDB backend (default, persistent, general purpose)
- `memory`: In-memory database (fastest, no persistence - uses temp directories)
- `nostrdb`: NostrDB backend (persistent, optimized for Nostr) [Not yet implemented]

> **Note:** When using the `memory` backend, git data are automatically stored in temporary directories for ephemeral testing.

### Example: Production Deployment

```bash
# Using environment variables (recommended for production)
export NGIT_DOMAIN=gitnostr.com
export NGIT_OWNER_NPUB=npub1...
export NGIT_BIND_ADDRESS=0.0.0.0:8080
export NGIT_DATABASE_BACKEND=lmdb
ngit-grasp
```

### Example: Development

```bash
# Using .env file
cp .env.example .env
# Edit .env with your settings
ngit-grasp

# Or override specific values with CLI flags
ngit-grasp --domain localhost:3000 --bind-address 127.0.0.1:3000
```

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
│   ├── lib.rs               # Library exports
│   ├── config.rs            # Configuration
│   ├── git/
│   │   ├── mod.rs           # Git module + repository operations
│   │   ├── handlers.rs      # Git HTTP handlers
│   │   ├── authorization.rs # Push validation logic
│   │   ├── protocol.rs      # Git protocol encoding
│   │   └── subprocess.rs    # Git subprocess management
│   ├── nostr/
│   │   ├── mod.rs           # Nostr module
│   │   ├── builder.rs       # Relay builder + Nip34WritePolicy
│   │   ├── events.rs        # Event parsing and validation
│   │   └── policy/          # Sub-policies (split for maintainability)
│   │       ├── mod.rs       # Policy module exports
│   │       ├── announcement.rs  # Repository announcement validation
│   │       ├── state.rs     # State event validation + ref alignment
│   │       ├── pr_event.rs  # PR/PR Update validation
│   │       └── related.rs   # Forward/backward reference checking
│   ├── http/
│   │   ├── mod.rs           # HTTP module
│   │   ├── landing.rs       # Landing page handler
│   │   └── nip11.rs         # NIP-11 relay info document
│   └── metrics/
│       ├── mod.rs           # Prometheus metrics
│       ├── bandwidth.rs     # Bandwidth tracking
│       └── connection.rs    # Connection tracking
├── docs/                    # Documentation (Diátaxis framework)
├── tests/                   # Integration tests
├── grasp-audit/             # Compliance audit subproject
└── README.md
```

## Comparison with ngit-relay

| Feature       | ngit-relay (Go)                           | ngit-grasp (Rust)             |
| ------------- | ----------------------------------------- | ----------------------------- |
| Language      | Go                                        | Rust                          |
| Components    | nginx + git-http-backend + hooks + Khatru | Single integrated binary      |
| Authorization | Pre-receive Git hook                      | Inline during receive-pack    |
| Deployment    | Docker + supervisord                      | Single binary                 |
| Testing       | Go tests + shell scripts                  | Rust unit + integration tests |
| Performance   | Good                                      | Excellent (zero-copy, async)  |

## Contributing

Contributions welcome! Please:

1. Read [docs/explanation/architecture.md](docs/ARCHITECTURE.md)
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
