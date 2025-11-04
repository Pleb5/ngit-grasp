# GRASP Audit Tool - Revised Plan

**Decision:** Option B - Parallel development with separate `grasp-audit` crate

## Key Requirements

1. ✅ **Separate crate**: `grasp-audit` (not `grasp-compliance-tests`)
2. ✅ **Parallel development**: Build ngit-grasp and tests simultaneously
3. ✅ **Isolated tests**: Can run in parallel for CI/CD
4. ✅ **Production audit**: Can test live production services
5. ✅ **Clean audit events**: Use special tags for easy cleanup (no deletion events)

## Audit Event Strategy

### The Challenge

Tests create events on the relay. We need to:
- Identify audit events vs. real events
- Clean them up without leaving deletion trails
- Support both isolated CI/CD tests and production audits

### Solution: Audit Tags

**Every audit event includes a special tag:**

```json
{
  "tags": [
    ["grasp-audit", "true"],
    ["audit-run-id", "ci-2025-11-03-12345"],
    ["audit-cleanup", "2025-11-03T12:00:00Z"]
  ]
}
```

**Tag meanings:**
- `grasp-audit: true` - Marks this as an audit event
- `audit-run-id` - Unique ID for this test run (for isolation)
- `audit-cleanup` - Timestamp after which this can be cleaned up

### Cleanup Script

```bash
# grasp-audit-cleanup.sh
# Run this periodically to clean up old audit events

grasp-audit cleanup \
  --relay ws://localhost:7000 \
  --older-than 24h \
  --dry-run  # Remove for actual cleanup
```

The cleanup script:
1. Queries for events with `grasp-audit: true`
2. Checks `audit-cleanup` timestamp
3. Deletes events older than threshold
4. No deletion events - direct database cleanup

### Test Isolation

**CI/CD Mode:**
```rust
// Each test run gets unique ID
let audit_id = format!("ci-{}-{}", 
    env::var("CI_RUN_ID").unwrap_or_default(),
    Uuid::new_v4()
);

// Tests only query their own events
let filter = Filter::new()
    .custom_tag(SingleLetterTag::lowercase(Alphabet::A), ["true"])
    .custom_tag(SingleLetterTag::lowercase(Alphabet::B), [&audit_id]);
```

**Production Audit Mode:**
```rust
// Production audits use timestamped IDs
let audit_id = format!("prod-audit-{}", Utc::now().timestamp());

// Query all events (including real ones) to verify production behavior
let filter = Filter::new()
    .kind(Kind::Custom(30617));  // No audit filter - test real state
```

## Project Structure

```
grasp-audit/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs                    # Public API
│   ├── client.rs                 # Test client
│   ├── audit.rs                  # Audit event handling
│   ├── cleanup.rs                # Cleanup utilities
│   ├── isolation.rs              # Test isolation helpers
│   └── specs/
│       ├── mod.rs
│       ├── nip01_smoke.rs        # 6 smoke tests
│       └── grasp_01_relay.rs     # 12 GRASP tests
├── fixtures/
│   ├── repos/
│   ├── events/
│   └── keys/
├── examples/
│   ├── audit_server.rs           # Audit a running server
│   └── ci_tests.rs               # CI/CD isolated tests
└── bin/
    └── grasp-audit.rs            # CLI tool for cleanup
```

## Audit Client Design

```rust
// src/audit.rs

use nostr_sdk::prelude::*;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AuditConfig {
    /// Unique ID for this audit run
    pub run_id: String,
    
    /// Mode: CI (isolated) or Production (live)
    pub mode: AuditMode,
    
    /// Cleanup timestamp (events can be cleaned after this)
    pub cleanup_after: Timestamp,
    
    /// Whether to actually create events or just query
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditMode {
    /// Isolated CI/CD tests - only see own events
    CI,
    
    /// Production audit - see all events, minimal writes
    Production,
}

impl AuditConfig {
    /// Create config for CI/CD testing
    pub fn ci() -> Self {
        let run_id = format!("ci-{}", uuid::Uuid::new_v4());
        Self {
            run_id,
            mode: AuditMode::CI,
            cleanup_after: Timestamp::now() + Duration::from_secs(3600), // 1 hour
            read_only: false,
        }
    }
    
    /// Create config for production audit
    pub fn production() -> Self {
        let run_id = format!("prod-audit-{}", Timestamp::now().as_u64());
        Self {
            run_id,
            mode: AuditMode::Production,
            cleanup_after: Timestamp::now() + Duration::from_secs(300), // 5 minutes
            read_only: true, // Default to read-only for production
        }
    }
}

/// Wrapper that adds audit tags to all events
pub struct AuditClient {
    client: Client,
    config: AuditConfig,
}

impl AuditClient {
    pub async fn new(relay_url: &str, config: AuditConfig) -> Result<Self> {
        let client = Client::new(&Keys::generate());
        client.add_relay(relay_url).await?;
        client.connect().await;
        
        Ok(Self { client, config })
    }
    
    /// Send an event with audit tags
    pub async fn send_event(&self, mut event: Event) -> Result<EventId> {
        if self.config.read_only {
            return Err(anyhow!("Client is in read-only mode"));
        }
        
        // Add audit tags
        event = self.add_audit_tags(event)?;
        
        let event_id = self.client.send_event(event).await?;
        Ok(event_id)
    }
    
    /// Query events, optionally filtered to this audit run
    pub async fn query(&self, mut filter: Filter) -> Result<Vec<Event>> {
        if self.config.mode == AuditMode::CI {
            // In CI mode, only see our own audit events
            filter = filter
                .custom_tag(
                    SingleLetterTag::lowercase(Alphabet::A), 
                    ["true"]
                )
                .custom_tag(
                    SingleLetterTag::lowercase(Alphabet::B), 
                    [&self.config.run_id]
                );
        }
        // In Production mode, see all events (no filter modification)
        
        let events = self.client
            .get_events_of(vec![filter], Some(Duration::from_secs(10)))
            .await?;
        
        Ok(events)
    }
    
    fn add_audit_tags(&self, event: Event) -> Result<Event> {
        // This is tricky - we need to rebuild the event with new tags
        // For now, we'll require events to be built through our builder
        
        // TODO: Implement event tag injection
        // This requires re-signing the event, which needs the private key
        
        Ok(event)
    }
}

/// Builder for audit events
pub struct AuditEventBuilder {
    builder: EventBuilder,
    config: AuditConfig,
}

impl AuditEventBuilder {
    pub fn new(kind: Kind, content: impl Into<String>, config: AuditConfig) -> Self {
        Self {
            builder: EventBuilder::new(kind, content, []),
            config,
        }
    }
    
    pub fn tag(mut self, tag: Tag) -> Self {
        self.builder = self.builder.add_tags(vec![tag]);
        self
    }
    
    pub fn tags(mut self, tags: Vec<Tag>) -> Self {
        self.builder = self.builder.add_tags(tags);
        self
    }
    
    pub async fn build(mut self, keys: &Keys) -> Result<Event> {
        // Add audit tags
        let audit_tags = vec![
            Tag::custom(
                TagKind::Custom(std::borrow::Cow::Borrowed("grasp-audit")),
                vec!["true"]
            ),
            Tag::custom(
                TagKind::Custom(std::borrow::Cow::Borrowed("audit-run-id")),
                vec![&self.config.run_id]
            ),
            Tag::custom(
                TagKind::Custom(std::borrow::Cow::Borrowed("audit-cleanup")),
                vec![&self.config.cleanup_after.to_string()]
            ),
        ];
        
        self.builder = self.builder.add_tags(audit_tags);
        
        Ok(self.builder.to_event(keys).await?)
    }
}
```

## Test Structure with Isolation

```rust
// src/specs/nip01_smoke.rs

use crate::*;

pub struct Nip01SmokeTests;

impl Nip01SmokeTests {
    pub async fn run_all(client: &AuditClient) -> AuditResult {
        let mut results = AuditResult::new("NIP-01 Smoke Tests");
        
        // All tests run in parallel with isolated audit IDs
        let tests = vec![
            Self::test_websocket_connection(client),
            Self::test_send_receive_event(client),
            Self::test_create_subscription(client),
            Self::test_close_subscription(client),
            Self::test_reject_invalid_event(client),
            Self::test_reject_invalid_event_id(client),
        ];
        
        let test_results = futures::future::join_all(tests).await;
        
        for result in test_results {
            results.add(result);
        }
        
        results
    }
    
    async fn test_websocket_connection(client: &AuditClient) -> TestResult {
        TestResult::new(
            "websocket_connection",
            "NIP-01:basic",
            "Can establish WebSocket connection to /",
        )
        .run(async {
            // Test connection
            client.client.connect().await;
            
            // Verify connected
            if !client.client.is_connected() {
                return Err("Failed to connect to relay".into());
            }
            
            Ok(())
        })
        .await
    }
    
    async fn test_send_receive_event(client: &AuditClient) -> TestResult {
        TestResult::new(
            "send_receive_event",
            "NIP-01:event-message",
            "Can send EVENT and receive OK response",
        )
        .run(async {
            let keys = Keys::generate();
            
            // Create audit event
            let event = AuditEventBuilder::new(
                Kind::TextNote,
                "Test event for smoke test",
                client.config.clone(),
            )
            .build(&keys)
            .await?;
            
            // Send event
            let event_id = client.send_event(event).await?;
            
            // Query it back (in CI mode, only sees our events)
            let filter = Filter::new()
                .kind(Kind::TextNote)
                .id(event_id);
            
            let events = client.query(filter).await?;
            
            if events.is_empty() {
                return Err("Event not found after sending".into());
            }
            
            Ok(())
        })
        .await
    }
    
    // ... other tests
}
```

## CLI Tool for Cleanup

```rust
// bin/grasp-audit.rs

use clap::{Parser, Subcommand};
use grasp_audit::*;

#[derive(Parser)]
#[command(name = "grasp-audit")]
#[command(about = "GRASP audit and cleanup tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run audit tests against a server
    Audit {
        /// Relay URL
        #[arg(short, long)]
        relay: String,
        
        /// Mode: ci or production
        #[arg(short, long, default_value = "ci")]
        mode: String,
        
        /// Spec to test (nip01-smoke, grasp-01-relay, all)
        #[arg(short, long, default_value = "all")]
        spec: String,
    },
    
    /// Clean up old audit events
    Cleanup {
        /// Relay URL
        #[arg(short, long)]
        relay: String,
        
        /// Delete events older than this (e.g., "24h", "7d")
        #[arg(short, long, default_value = "24h")]
        older_than: String,
        
        /// Dry run (don't actually delete)
        #[arg(short, long)]
        dry_run: bool,
    },
    
    /// List audit events
    List {
        /// Relay URL
        #[arg(short, long)]
        relay: String,
        
        /// Filter by run ID
        #[arg(short = 'i', long)]
        run_id: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Audit { relay, mode, spec } => {
            let config = match mode.as_str() {
                "ci" => AuditConfig::ci(),
                "production" => AuditConfig::production(),
                _ => return Err(anyhow!("Invalid mode: {}", mode)),
            };
            
            let client = AuditClient::new(&relay, config).await?;
            
            println!("Running audit in {} mode...", mode);
            println!("Audit run ID: {}", client.config.run_id);
            
            let results = match spec.as_str() {
                "nip01-smoke" => Nip01SmokeTests::run_all(&client).await,
                "grasp-01-relay" => Grasp01RelayTests::run_all(&client).await,
                "all" => {
                    let mut all = AuditResult::new("All Tests");
                    all.merge(Nip01SmokeTests::run_all(&client).await);
                    all.merge(Grasp01RelayTests::run_all(&client).await);
                    all
                }
                _ => return Err(anyhow!("Unknown spec: {}", spec)),
            };
            
            results.print_report();
            
            if !results.all_passed() {
                std::process::exit(1);
            }
        }
        
        Commands::Cleanup { relay, older_than, dry_run } => {
            println!("Cleaning up audit events from {}...", relay);
            
            let duration = parse_duration(&older_than)?;
            let cutoff = Timestamp::now() - duration;
            
            let client = Client::new(&Keys::generate());
            client.add_relay(&relay).await?;
            client.connect().await;
            
            // Query audit events
            let filter = Filter::new()
                .custom_tag(
                    SingleLetterTag::lowercase(Alphabet::A),
                    ["true"]
                );
            
            let events = client
                .get_events_of(vec![filter], Some(Duration::from_secs(10)))
                .await?;
            
            let mut deleted = 0;
            
            for event in events {
                // Check cleanup timestamp
                let cleanup_tag = event.tags.iter()
                    .find(|t| t.kind() == TagKind::Custom("audit-cleanup".into()));
                
                if let Some(tag) = cleanup_tag {
                    if let Some(timestamp_str) = tag.content() {
                        let cleanup_time = Timestamp::from_str(timestamp_str)?;
                        
                        if cleanup_time < cutoff {
                            if dry_run {
                                println!("Would delete: {} ({})", 
                                    event.id, 
                                    event.created_at
                                );
                            } else {
                                // TODO: Implement direct database deletion
                                // For now, we can't delete without NIP-09 deletion events
                                println!("Delete: {} ({})", 
                                    event.id, 
                                    event.created_at
                                );
                            }
                            deleted += 1;
                        }
                    }
                }
            }
            
            println!("\n{} events cleaned up", deleted);
            if dry_run {
                println!("(dry run - no actual deletion)");
            }
        }
        
        Commands::List { relay, run_id } => {
            let client = Client::new(&Keys::generate());
            client.add_relay(&relay).await?;
            client.connect().await;
            
            let mut filter = Filter::new()
                .custom_tag(
                    SingleLetterTag::lowercase(Alphabet::A),
                    ["true"]
                );
            
            if let Some(id) = run_id {
                filter = filter.custom_tag(
                    SingleLetterTag::lowercase(Alphabet::B),
                    [id]
                );
            }
            
            let events = client
                .get_events_of(vec![filter], Some(Duration::from_secs(10)))
                .await?;
            
            println!("Found {} audit events:\n", events.len());
            
            for event in events {
                let run_id = event.tags.iter()
                    .find(|t| t.kind() == TagKind::Custom("audit-run-id".into()))
                    .and_then(|t| t.content())
                    .unwrap_or("unknown");
                
                println!("ID: {}", event.id);
                println!("  Run: {}", run_id);
                println!("  Kind: {}", event.kind);
                println!("  Created: {}", event.created_at);
                println!();
            }
        }
    }
    
    Ok(())
}

fn parse_duration(s: &str) -> Result<Duration> {
    // Simple parser for "24h", "7d", etc.
    let (num, unit) = s.split_at(s.len() - 1);
    let num: u64 = num.parse()?;
    
    let seconds = match unit {
        "s" => num,
        "m" => num * 60,
        "h" => num * 3600,
        "d" => num * 86400,
        _ => return Err(anyhow!("Invalid duration unit: {}", unit)),
    };
    
    Ok(Duration::from_secs(seconds))
}
```

## Usage Examples

### CI/CD Mode (Isolated Tests)

```bash
# Run in CI - each run is isolated
grasp-audit audit --relay ws://localhost:7000 --mode ci --spec all

# Output:
# Running audit in ci mode...
# Audit run ID: ci-a1b2c3d4-e5f6-7890-abcd-ef1234567890
# 
# NIP-01 Smoke Tests
# ══════════════════════════════════════════════════════════
# ✓ websocket_connection (NIP-01:basic)
# ✓ send_receive_event (NIP-01:event-message)
# ...
# Results: 6/6 passed
```

### Production Audit Mode

```bash
# Audit production server (read-only by default)
grasp-audit audit \
  --relay wss://relay.example.com \
  --mode production \
  --spec grasp-01-relay

# Output:
# Running audit in production mode...
# Audit run ID: prod-audit-1699027200
# 
# GRASP-01: Relay Requirements
# ══════════════════════════════════════════════════════════
# ✓ accepts_repository_announcement (GRASP-01:9-10)
# ✗ rejects_announcement_without_clone_tag (GRASP-01:12-13)
#   Error: Production relay accepted invalid announcement
# ...
```

### Cleanup

```bash
# List all audit events
grasp-audit list --relay ws://localhost:7000

# Dry run cleanup
grasp-audit cleanup \
  --relay ws://localhost:7000 \
  --older-than 24h \
  --dry-run

# Actual cleanup
grasp-audit cleanup \
  --relay ws://localhost:7000 \
  --older-than 24h
```

## Parallel Development Plan

### Week 1: Foundation (Both in Parallel)

**grasp-audit:**
- Day 1: Create crate structure
- Day 2: Implement AuditClient with tag injection
- Day 3: Implement 6 smoke tests
- Day 4: Implement CLI tool skeleton
- Day 5: Test isolation and cleanup

**ngit-grasp:**
- Day 1: Create project structure
- Day 2: Set up nostr-relay-builder
- Day 3: Basic relay serving at /
- Day 4: NIP-11 document
- Day 5: Event acceptance (no policy yet)

### Week 2: Integration

**grasp-audit:**
- Day 1-2: Implement GRASP-01 relay tests
- Day 3: Fixtures and builders
- Day 4-5: Documentation and examples

**ngit-grasp:**
- Day 1-2: Implement GRASP policy (clone/relay tags)
- Day 3: Related event acceptance
- Day 4-5: Fix failing audit tests

### Week 3-4: Iteration

Run audit tests continuously, fix issues, iterate until all pass.

## Next Steps

1. ✅ Create `grasp-audit/` crate structure
2. ✅ Implement AuditClient with tag injection
3. ✅ Implement first smoke test
4. ✅ Test it against a simple relay
5. ✅ Report back with results

Let me start with the implementation...
