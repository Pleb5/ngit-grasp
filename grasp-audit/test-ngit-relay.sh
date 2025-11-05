#!/bin/bash
set -e

echo "🧹 Cleaning up old test data..."
# Use docker to cleanup with proper permissions
docker run --rm -v /tmp/ngit-test:/data alpine sh -c "rm -rf /data/*" 2>/dev/null || true
mkdir -p /tmp/ngit-test/{repos,blossom,relay-db,logs}

echo "🚀 Starting ngit-relay..."
# Remove any existing container with this name
docker rm -f ngit-relay-test 2>/dev/null || true
docker run --rm -d \
  --name ngit-relay-test \
  -p 8082:8081 \
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

echo "⏳ Waiting for relay to start..."
sleep 3

echo "🧪 Running tests..."
echo ""
echo "Note: ngit-relay only accepts Git-related events (NIP-34)."
echo "Some NIP-01 smoke tests will fail (expected behavior)."
echo "Validation tests should pass."
echo ""
RELAY_URL=ws://localhost:8082 cargo test --lib -- --ignored --nocapture

echo "🛑 Stopping relay..."
docker stop ngit-relay-test

echo "🧹 Cleaning up..."
docker run --rm -v /tmp/ngit-test:/data alpine sh -c "rm -rf /data/*" 2>/dev/null || true

echo "✅ Done!"
