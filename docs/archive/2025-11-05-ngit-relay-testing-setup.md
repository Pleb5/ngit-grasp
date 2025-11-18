# ngit-relay Testing Setup - COMPLETE

**Date:** November 5, 2025  
**Status:** ✅ COMPLETE  
**Purpose:** Document how to test grasp-audit against ngit-relay reference implementation

---

## ✅ What Was Done

### 1. Updated grasp-audit/README.md

Added comprehensive section "Integration Tests Against ngit-relay" with:

- **Step-by-step manual instructions** for running tests
- **Environment variable explanations** (all required vars documented)
- **Port mapping details** (both WebSocket and HTTP on 8081)
- **Clean state strategy** (fresh /tmp directories for each run)
- **Cleanup procedures** (stop container, remove test data)

### 2. Created test-ngit-relay.sh Script

Automated test script at `grasp-audit/test-ngit-relay.sh` that:

- ✅ Creates fresh test directories in /tmp
- ✅ Starts ngit-relay Docker container with correct env vars
- ✅ Waits for relay to start (3 second delay)
- ✅ Runs integration tests (`cargo test --ignored`)
- ✅ Stops container
- ✅ Cleans up test data
- ✅ Executable permissions set (`chmod +x`)
- ✅ Syntax validated

---

## 🔑 Key Information

### Docker Image
```
ghcr.io/danconwaydev/ngit-relay:latest
```

### Required Environment Variables
```bash
NGIT_DOMAIN=localhost                    # Domain name
NGIT_RELAY_NAME="ngit-relay test instance"
NGIT_RELAY_DESCRIPTION="Test instance for grasp-audit"
NGIT_OWNER_NPUB="npub15qydau2hjma6ngxkl2cyar74wzyjshvl65za5k5rl69264ar2exs5cyejr"
NGIT_PROACTIVE_SYNC_GIT=false           # Disable for testing
NGIT_PROACTIVE_SYNC_BLOSSOM=false       # Disable for testing
NGIT_PROACTIVE_SYNC_NOSTR=false         # Disable for testing
NGIT_LOG_LEVEL=INFO                     # For debugging
```

### Volume Mounts (Fresh for Each Run)
```bash
/tmp/ngit-test/repos       → /srv/ngit-relay/repos
/tmp/ngit-test/blossom     → /srv/ngit-relay/blossom
/tmp/ngit-test/relay-db    → /srv/ngit-relay/relay-db
/tmp/ngit-test/logs        → /var/log/ngit-relay
```

### Port Mapping
```
8081:8081  # Both WebSocket (relay) and HTTP (git) on same port
```

### Endpoints
- **WebSocket (Nostr relay):** `ws://localhost:8081/`
- **Git HTTP:** `http://localhost:8081/<npub>/<identifier>.git`

---

## 🎯 Usage

### Option 1: Manual Commands

```bash
cd grasp-audit

# 1. Create temp directories
mkdir -p /tmp/ngit-test/{repos,blossom,relay-db,logs}

# 2. Start relay
docker run --rm -d \
  --name ngit-relay-test \
  -p 8081:8081 \
  -e NGIT_DOMAIN=localhost \
  -e NGIT_RELAY_NAME="ngit-relay test instance" \
  -e NGIT_RELAY_DESCRIPTION="Test instance for grasp-audit" \
  -e NGIT_OWNER_NPUB="npub15qydau2hjma6ngxkl2cyar74wzyjshvl65za5k5rl69264ar2exs5cyejr" \
  -e NGIT_PROACTIVE_SYNC_GIT=false \
  -e NGIT_PROACTIVE_SYNC_BLOSSOM=false \
  -e NGIT_PROACTIVE_SYNC_NOSTR=false \
  -e NGIT_LOG_LEVEL=INFO \
  -v /tmp/ngit-test/repos:/srv/ngit-relay/repos \
  -v /tmp/ngit-test/blossom:/srv/ngit-relay/blossom \
  -v /tmp/ngit-test/relay-db:/srv/ngit-relay/relay-db \
  -v /tmp/ngit-test/logs:/var/log/ngit-relay \
  ghcr.io/danconwaydev/ngit-relay:latest

# 3. Wait for startup
sleep 3

# 4. Run tests
cargo test --ignored

# 5. Cleanup
docker stop ngit-relay-test
rm -rf /tmp/ngit-test
```

### Option 2: Quick Script

```bash
cd grasp-audit
./test-ngit-relay.sh
```

---

## 🧪 What Gets Tested

When you run `cargo test --ignored`, it runs integration tests that:

1. **Connect to the relay** at `ws://localhost:8081/`
2. **Verify NIP-01 compliance** (smoke tests)
3. **Test GRASP-01 features** (when implemented)
4. **Validate against reference implementation** behavior

---

## ✅ Benefits

### Clean State Every Run
- Fresh directories in /tmp
- No pollution from previous tests
- Matches CI environment

### Easy Debugging
- Manual commands for step-by-step debugging
- Automated script for quick validation
- Logs available in /tmp/ngit-test/logs

### Reference Implementation Testing
- Tests against the actual GRASP reference (ngit-relay)
- Ensures compatibility with real-world implementation
- Validates our tests match expected behavior

---

## 📚 References

- **ngit-relay repo:** `../ngit-relay`
- **Docker image:** `ghcr.io/danconwaydev/ngit-relay:latest`
- **Environment vars:** `../ngit-relay/.env.example`
- **Documentation:** `../ngit-relay/README.md`

---

## 🔜 Next Steps

Now that we can test against ngit-relay, we're ready to:

1. ✅ **Verify current NIP-01 smoke tests work** against ngit-relay
2. 🔜 **Implement GRASP-01 tests** one at a time (per plan in work/current_status.md)
3. 🔜 **Validate each test** against reference implementation
4. 🔜 **Document any behavioral differences** we discover

---

**Ready to proceed with test implementation!**

The plan in `work/current_status.md` calls for implementing GRASP-01 tests one at a time, each in a fresh session, validating against ngit-relay.

We now have the infrastructure to do exactly that. ✅
