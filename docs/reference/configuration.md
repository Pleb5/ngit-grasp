# Reference: Configuration

**Purpose:** Complete reference for all ngit-grasp configuration options  
**Audience:** Operators and developers

---

## Configuration Methods

ngit-grasp can be configured via:

1. **Environment variables** (recommended for deployment)
2. **`.env` file** (recommended for development)
3. **Command-line arguments** (planned, not yet implemented)

Configuration is loaded at startup and validated before the server starts.

---

## Environment Variables

### Server Configuration

#### `NGIT_BIND_ADDRESS`

**Description:** Address and port for the HTTP server to bind to  
**Type:** String (IP:PORT format)  
**Default:** `127.0.0.1:8080`  
**Required:** No

**Examples:**
```bash
# Localhost only (development)
NGIT_BIND_ADDRESS=127.0.0.1:8080

# All interfaces (production)
NGIT_BIND_ADDRESS=0.0.0.0:8080

# IPv6
NGIT_BIND_ADDRESS=[::1]:8080

# Custom port
NGIT_BIND_ADDRESS=127.0.0.1:3000
```

**Notes:**
- Use `127.0.0.1` for local development
- Use `0.0.0.0` for production (behind reverse proxy)
- Ensure firewall rules allow the port

---

#### `NGIT_DOMAIN`

**Description:** Public domain name for this GRASP instance  
**Type:** String (domain name)  
**Default:** None  
**Required:** Yes

**Examples:**
```bash
NGIT_DOMAIN=gitnostr.com
NGIT_DOMAIN=git.example.org
NGIT_DOMAIN=localhost:8080  # Development only
```

**Used for:**
- NIP-11 relay information document
- Generating repository URLs
- CORS configuration
- Webhook URLs (future)

**Notes:**
- Must be accessible from the internet for production
- Include port if non-standard (e.g., `localhost:8080`)
- Used in repository clone URLs: `https://{NGIT_DOMAIN}/{npub}/{repo}.git`

---

### Nostr Relay Configuration

#### `NGIT_OWNER_NPUB`

**Description:** Nostr public key (npub format) of the relay operator  
**Type:** String (npub1... format)  
**Default:** None  
**Required:** Yes

**Examples:**
```bash
NGIT_OWNER_NPUB=npub1alice...
```

**Used for:**
- NIP-11 relay information document
- Contact information
- Administrative operations (future)

**Notes:**
- Must be valid npub format (starts with `npub1`)
- Can be generated with Nostr tools
- Publicly visible in relay metadata

---

#### `NGIT_RELAY_NAME`

**Description:** Human-readable name for this relay  
**Type:** String  
**Default:** `"ngit-grasp relay"`  
**Required:** No

**Examples:**
```bash
NGIT_RELAY_NAME="GitNostr Community Relay"
NGIT_RELAY_NAME="Alice's GRASP Server"
```

**Used for:**
- NIP-11 relay information document
- Client display
- Relay discovery

---

#### `NGIT_RELAY_DESCRIPTION`

**Description:** Description of this relay's purpose and policies  
**Type:** String  
**Default:** `"A GRASP-compliant Git relay"`  
**Required:** No

**Examples:**
```bash
NGIT_RELAY_DESCRIPTION="Public GRASP relay for open source projects"
NGIT_RELAY_DESCRIPTION="Private relay for ACME Corp repositories"
```

**Used for:**
- NIP-11 relay information document
- User information
- Relay selection

---

### Storage Configuration

#### `NGIT_GIT_DATA_PATH`

**Description:** Directory path for storing Git repositories  
**Type:** String (filesystem path)  
**Default:** `./data/git`  
**Required:** No

**Examples:**
```bash
# Relative path (development)
NGIT_GIT_DATA_PATH=./data/git

# Absolute path (production)
NGIT_GIT_DATA_PATH=/var/lib/ngit-grasp/git

# Custom location
NGIT_GIT_DATA_PATH=/mnt/storage/git-repos
```

**Storage structure:**
```
{NGIT_GIT_DATA_PATH}/
  ├── {npub1}/
  │   ├── {repo1}.git/
  │   │   ├── objects/
  │   │   ├── refs/
  │   │   └── ...
  │   └── {repo2}.git/
  └── {npub2}/
      └── ...
```

**Notes:**
- Directory must be writable by ngit-grasp process
- Ensure sufficient disk space
- Consider backup strategy
- Use fast storage for better performance

---

#### `NGIT_RELAY_DATA_PATH`

**Description:** Directory path for storing Nostr events and relay data  
**Type:** String (filesystem path)  
**Default:** `./data/relay`  
**Required:** No

**Examples:**
```bash
# Relative path (development)
NGIT_RELAY_DATA_PATH=./data/relay

# Absolute path (production)
NGIT_RELAY_DATA_PATH=/var/lib/ngit-grasp/relay

# Separate disk
NGIT_RELAY_DATA_PATH=/mnt/ssd/relay-data
```

**Storage structure:**
```
{NGIT_RELAY_DATA_PATH}/
  ├── events/
  │   └── {event-id}.json
  ├── indexes/
  │   ├── by-kind/
  │   ├── by-author/
  │   └── by-tag/
  └── metadata/
```

**Notes:**
- Directory must be writable
- Consider SSD for better query performance
- Size grows with event count
- Implement retention policy for production

---

#### `NGIT_DATABASE_BACKEND`

**Description:** Database backend type for storing Nostr events
**Type:** String (enum: memory, nostrdb, lmdb)
**Default:** `memory`
**Required:** No

**Valid Values:**
- `memory` - In-memory database (default, fastest, no persistence)
- `nostrdb` - NostrDB backend (persistent, optimized for Nostr) [Not yet implemented]
- `lmdb` - LMDB backend (persistent, general purpose) [Not yet implemented]

**Examples:**
```bash
# Development (default, no persistence)
NGIT_DATABASE_BACKEND=memory

# Production with NostrDB (when implemented)
NGIT_DATABASE_BACKEND=nostrdb

# Production with LMDB (when implemented)
NGIT_DATABASE_BACKEND=lmdb
```

**Comparison:**

| Backend | Persistence | Performance | Use Case |
|---------|-------------|-------------|----------|
| memory | No | Fastest | Development, testing |
| nostrdb | Yes | High | Production (Nostr-optimized) |
| lmdb | Yes | High | Production (general purpose) |

**Notes:**
- `memory` backend loses all data on restart
- NostrDB and LMDB backends will use `NGIT_RELAY_DATA_PATH` for storage
- NostrDB and LMDB are planned features, not yet available
- Default `memory` backend suitable for development and testing only
- Production deployments should use persistent backends when available

---

### Logging Configuration

#### `RUST_LOG`

**Description:** Logging level and filters (standard Rust environment variable)  
**Type:** String (log level or filter)  
**Default:** `info`  
**Required:** No

**Examples:**
```bash
# Simple levels
RUST_LOG=error    # Errors only
RUST_LOG=warn     # Warnings and errors
RUST_LOG=info     # Info, warnings, errors
RUST_LOG=debug    # Debug and above
RUST_LOG=trace    # Everything

# Module-specific
RUST_LOG=ngit_grasp=debug,actix_web=info

# Complex filters
RUST_LOG=debug,hyper=info,tokio=warn
```

**Log levels (most to least verbose):**
1. `trace` - Very detailed, performance impact
2. `debug` - Detailed debugging information
3. `info` - General information (default)
4. `warn` - Warnings about potential issues
5. `error` - Errors only

**Production recommendation:**
```bash
RUST_LOG=info,ngit_grasp=debug
```

---

### Security Configuration (Planned)

#### `NGIT_AUTH_REQUIRED`

**Description:** Require authentication for all operations  
**Type:** Boolean  
**Default:** `false`  
**Status:** 🔜 Planned

**Examples:**
```bash
NGIT_AUTH_REQUIRED=true   # Require auth
NGIT_AUTH_REQUIRED=false  # Public relay
```

---

#### `NGIT_RATE_LIMIT_ENABLED`

**Description:** Enable rate limiting  
**Type:** Boolean  
**Default:** `true`  
**Status:** 🔜 Planned

**Examples:**
```bash
NGIT_RATE_LIMIT_ENABLED=true
NGIT_RATE_LIMIT_ENABLED=false
```

---

## Configuration File (.env)

For development, create a `.env` file in the project root:

```bash
# .env file example
NGIT_DOMAIN=localhost:8080
NGIT_OWNER_NPUB=npub1alice...
NGIT_RELAY_NAME="Development Relay"
NGIT_RELAY_DESCRIPTION="Local development instance"
NGIT_GIT_DATA_PATH=./data/git
NGIT_RELAY_DATA_PATH=./data/relay
NGIT_BIND_ADDRESS=127.0.0.1:8080
RUST_LOG=debug
```

**Notes:**
- Never commit `.env` to version control
- Use `.env.example` as a template
- Environment variables override `.env` values

---

## Validation

Configuration is validated at startup:

```rust
// Example validation errors:
Error: Invalid configuration
  - NGIT_DOMAIN is required
  - NGIT_OWNER_NPUB must start with 'npub1'
  - NGIT_GIT_DATA_PATH is not writable
```

**Validation checks:**
- Required fields are present
- Values have correct format
- Paths are accessible and writable
- Ports are available
- npub keys are valid

---

## Production Configuration Example

```bash
# Production .env
NGIT_DOMAIN=gitnostr.com
NGIT_OWNER_NPUB=npub1alice...
NGIT_RELAY_NAME="GitNostr Public Relay"
NGIT_RELAY_DESCRIPTION="Public GRASP relay for open source projects"
NGIT_GIT_DATA_PATH=/var/lib/ngit-grasp/git
NGIT_RELAY_DATA_PATH=/var/lib/ngit-grasp/relay
NGIT_BIND_ADDRESS=0.0.0.0:8080
RUST_LOG=info,ngit_grasp=debug
```

**Additional production considerations:**
- Use reverse proxy (nginx, Caddy) for HTTPS
- Set up log rotation
- Configure monitoring
- Implement backup strategy
- Use dedicated user account
- Set file permissions properly

---

## Development Configuration Example

```bash
# Development .env
NGIT_DOMAIN=localhost:8080
NGIT_OWNER_NPUB=npub1test...
NGIT_RELAY_NAME="Dev Relay"
NGIT_RELAY_DESCRIPTION="Local development"
NGIT_GIT_DATA_PATH=./data/git
NGIT_RELAY_DATA_PATH=./data/relay
NGIT_BIND_ADDRESS=127.0.0.1:8080
RUST_LOG=debug
```

---

## Testing Configuration Example

```bash
# Testing .env
NGIT_DOMAIN=localhost:9999
NGIT_OWNER_NPUB=npub1test...
NGIT_RELAY_NAME="Test Relay"
NGIT_RELAY_DESCRIPTION="Automated testing"
NGIT_GIT_DATA_PATH=/tmp/ngit-test/git
NGIT_RELAY_DATA_PATH=/tmp/ngit-test/relay
NGIT_BIND_ADDRESS=127.0.0.1:9999
RUST_LOG=debug
```

**Testing notes:**
- Use temporary directories
- Use non-standard ports
- Clean up after tests
- Isolate from development data

---

## Configuration Priority

When multiple configuration sources exist:

1. **Command-line arguments** (highest priority, planned)
2. **Environment variables**
3. **`.env` file**
4. **Default values** (lowest priority)

**Example:**
```bash
# .env file
NGIT_BIND_ADDRESS=127.0.0.1:8080

# Environment variable (overrides .env)
NGIT_BIND_ADDRESS=0.0.0.0:3000 cargo run

# Result: binds to 0.0.0.0:3000
```

---

## Related Documentation

- [Deployment How-To](../how-to/deploy.md) - Production deployment
- [Getting Started Tutorial](../tutorials/getting-started.md) - Initial setup
- [Architecture Overview](../explanation/architecture.md) - System design

---

*Part of the [ngit-grasp reference documentation](./)*
