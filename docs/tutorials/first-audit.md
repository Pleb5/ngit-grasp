# Tutorial: Running Your First GRASP Audit

**Purpose:** Learn how to use grasp-audit to check GRASP compliance  
**Time:** 10-15 minutes  
**Prerequisites:** [Getting Started Tutorial](getting-started.md) completed

---

## What You'll Learn

By the end of this tutorial, you will:
- ✅ Understand what GRASP compliance means
- ✅ Run a compliance audit against a relay
- ✅ Interpret audit results
- ✅ Know how to use the audit tool in your own projects

---

## Step 1: Understanding GRASP Compliance

GRASP (Git Relays Authorized via Signed-Nostr Proofs) defines requirements for Git hosting with Nostr authorization.

**Key compliance areas:**
- **NIP-01**: Basic Nostr relay functionality
- **NIP-34**: Git repository events (kind 30317, 30318)
- **Git HTTP**: Smart HTTP protocol support
- **Authorization**: Push validation against state events

The `grasp-audit` tool verifies all of these automatically.

---

## Step 2: Start a Test Relay

For this tutorial, we'll use a standard Nostr relay:

```bash
# In a separate terminal window:
docker run --rm -p 7000:7000 scsibug/nostr-rs-relay

# Keep this running throughout the tutorial
```

**What this does:** Starts a NIP-01 compliant Nostr relay on port 7000.

**Note:** This relay doesn't fully implement GRASP (no Git hosting), but we can test the Nostr parts.

---

## Step 3: Run the Audit Tool

Navigate to the grasp-audit directory and run:

```bash
cd grasp-audit
nix develop

# Run the integration tests (which include audits)
cargo test --ignored -- --test-threads=1
```

**What you'll see:**
```
running 3 tests
test tests::test_isolation_basic ... ok
test tests::test_isolation_cleanup ... ok  
test tests::test_isolation_concurrent ... ok

test result: ok. 3 passed; 0 failed; 0 ignored
```

**What just happened?** The audit tool:
1. Connected to the relay on port 7000
2. Checked NIP-01 compliance (event submission, retrieval)
3. Tested isolation between test runs
4. Verified cleanup mechanisms

---

## Step 4: Use the Audit Library

Let's write a simple audit script. Create a new file:

```bash
# From grasp-audit directory
cat > examples/my_audit.rs << 'EOF'
use grasp_audit::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create an audit client
    let client = AuditClient::new("ws://localhost:7000").await?;
    
    println!("✅ Connected to relay");
    
    // Test basic event submission
    let test_event = client.create_test_event("Hello GRASP!").await?;
    println!("✅ Created test event: {}", test_event.id);
    
    // Verify we can retrieve it
    let retrieved = client.get_event(&test_event.id).await?;
    println!("✅ Retrieved event successfully");
    
    println!("\n🎉 Basic audit passed!");
    
    Ok(())
}
EOF
```

**Note:** This is a simplified example. The actual audit tool has more sophisticated checks.

---

## Step 5: Understanding Audit Results

When audits fail, you'll see detailed error messages:

```rust
// Example failure output:
Error: GRASP-01 compliance failed
  - NIP-01: ✅ PASS
  - NIP-34 kind 30317: ❌ FAIL - Relay rejected repository announcement
  - NIP-34 kind 30318: ❌ FAIL - Relay rejected state event
  - Git HTTP: ❌ NOT TESTED - No Git endpoint found
```

**How to interpret:**
- ✅ **PASS**: Feature works correctly
- ❌ **FAIL**: Feature broken or missing
- ⚠️ **PARTIAL**: Works but with issues
- ⏭️ **SKIPPED**: Couldn't test (dependency failed)

---

## Step 6: Audit a GRASP-Compliant Relay

To audit a real GRASP relay (when available):

```bash
# Example (relay doesn't exist yet):
cargo run --bin grasp-audit -- --relay wss://gitnostr.com

# Or use the library:
let client = AuditClient::new("wss://gitnostr.com").await?;
let results = client.run_full_audit().await?;
println!("{}", results.summary());
```

**What this would check:**
- Nostr relay functionality (NIP-01)
- Git event acceptance (NIP-34)
- Git HTTP endpoint availability
- Push authorization logic
- Multi-maintainer support

---

## Step 7: Automated Testing

The audit tool is designed for CI/CD integration:

```bash
# Run all tests (unit + integration)
cargo test --all

# Run only integration tests
cargo test --ignored

# Generate coverage report
cargo tarpaulin --ignored --out Html
```

**Use in CI:**
```yaml
# Example GitHub Actions
- name: Run GRASP Compliance Tests
  run: |
    docker run -d -p 7000:7000 scsibug/nostr-rs-relay
    cd grasp-audit
    cargo test --ignored
```

---

## What You've Accomplished

Congratulations! You now:

✅ Understand GRASP compliance requirements  
✅ Can run the audit tool against a relay  
✅ Know how to interpret audit results  
✅ Can integrate audits into your workflow

---

## Next Steps

### Learn more about testing:
- Read [Compliance Testing How-To](../how-to/test-compliance.md)
- Review [Test Strategy](../reference/test-strategy.md)

### Understand the protocols:
- Read [GRASP Protocol Reference](../reference/grasp-protocol.md)
- Review [Git Protocol Reference](../reference/git-protocol.md)

### Contribute to grasp-audit:
- Check open issues
- Add new compliance checks
- Improve error messages

---

## Troubleshooting

### "Connection refused" errors
- Make sure the relay is running: `docker ps`
- Check the port: `netstat -an | grep 7000`
- Verify the URL: `ws://localhost:7000` (not `wss://`)

### Tests timeout
- Relay might be slow to start
- Try running tests again after 5 seconds
- Check Docker logs: `docker logs <container-id>`

### "Event rejected" errors
- Expected for non-GRASP relays
- The relay might not support NIP-34
- This is normal for the tutorial relay

---

## Deep Dive: How Audits Work

The audit tool uses **isolated test environments**:

```rust
// Each test gets a unique identifier
let isolation = IsolationContext::new("my-test");

// Events are tagged with this identifier
let event = isolation.create_event("test content").await?;

// Cleanup removes only this test's events
isolation.cleanup().await?;
```

**Why isolation matters:**
- Tests don't interfere with each other
- Can run tests in parallel
- Easy cleanup (no leftover data)

See [Test Strategy Reference](../reference/test-strategy.md) for details.

---

## Summary

You've learned how to:
- Run GRASP compliance audits
- Interpret audit results
- Use the audit library
- Integrate audits into testing workflows

**Next tutorial:** [Deploying ngit-grasp](../how-to/deploy.md) (when main server is ready)

---

*Part of the [ngit-grasp tutorials](./)*  
*Previous: [Getting Started](getting-started.md)*
