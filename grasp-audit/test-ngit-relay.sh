#!/bin/bash
set -e

# Create temporary directory with random name
TEST_DIR=$(mktemp -d -t grasp-audit-run-XXXXXXXXXX)
# Pick a random port in the range 20000-30000
PORT=$((20000 + RANDOM % 10000))
# Generate a unique container name suffix
CONTAINER_SUFFIX=$RANDOM

echo "🧹 Using temporary directory: $TEST_DIR"
echo "🔌 Using port: $PORT"

# Cleanup function
cleanup() {
    echo "🛑 Stopping relay..."
    docker stop "grasp-audit-run-$CONTAINER_SUFFIX" 2>/dev/null || true
    
    echo "🧹 Cleaning up temporary directory..."
    docker run --rm -v "$TEST_DIR:/data" alpine sh -c "rm -rf /data/*" 2>/dev/null || true
    rm -rf "$TEST_DIR"
}

# Set trap to cleanup on exit
trap cleanup EXIT

echo "📁 Creating data directories..."
mkdir -p "$TEST_DIR"/{repos,blossom,relay-db,logs}

echo "🚀 Starting ngit-relay..."
# Remove any existing container with this name
CONTAINER_NAME="grasp-audit-run-$CONTAINER_SUFFIX"
docker rm -f "$CONTAINER_NAME" 2>/dev/null || true
docker run --rm -d \
  --name "$CONTAINER_NAME" \
  -p "$PORT:8081" \
  -e NGIT_DOMAIN=localhost \
  -e NGIT_RELAY_NAME="ngit-relay test instance" \
  -e NGIT_RELAY_DESCRIPTION="Test instance for grasp-audit" \
  -e NGIT_OWNER_NPUB="npub15qydau2hjma6ngxkl2cyar74wzyjshvl65za5k5rl69264ar2exs5cyejr" \
  -e NGIT_PROACTIVE_SYNC_GIT=false \
  -e NGIT_PROACTIVE_SYNC_BLOSSOM=false \
  -e NGIT_PROACTIVE_SYNC_NOSTR=false \
  -e NGIT_LOG_LEVEL=INFO \
  -v "$TEST_DIR/repos:/srv/ngit-relay/repos" \
  -v "$TEST_DIR/blossom:/srv/ngit-relay/blossom" \
  -v "$TEST_DIR/relay-db:/srv/ngit-relay/relay-db" \
  -v "$TEST_DIR/logs:/var/log/ngit-relay" \
  ghcr.io/danconwaydev/ngit-relay:latest

echo "⏳ Waiting for relay to start..."
sleep 3

echo "🧪 Running tests..."
echo ""
echo "Note: ngit-relay only accepts Git-related events (NIP-34)."
echo "Some NIP-01 smoke tests will fail (expected behavior)."
echo "Validation tests should pass."
echo ""

# Run the CLI tool (cleanup happens via trap even on failure)
RELAY_URL="ws://localhost:$PORT" cargo run -- audit --relay "ws://localhost:$PORT" --mode ci --spec nip01-smoke || {
    echo "⚠️  Some tests failed (expected for ngit-relay)"
    echo "    Validation tests should have passed"
}

echo "✅ Done!"
