#!/usr/bin/env bash
set -e

echo "🧪 Testing ngit-grasp NIP-01 relay"
echo ""

# Start the relay in the background
echo "📡 Starting relay on port 9000..."
cd grasp-audit
nix develop -c bash -c "cd .. && NGIT_BIND_ADDRESS=127.0.0.1:9000 RUST_LOG=info cargo run" > /tmp/relay.log 2>&1 &
RELAY_PID=$!
cd ..

echo "Relay PID: $RELAY_PID"
echo "Waiting for relay to start..."
sleep 3

# Check if relay is running
if ! ps -p $RELAY_PID > /dev/null; then
    echo "❌ Relay failed to start"
    cat /tmp/relay.log
    exit 1
fi

echo "✅ Relay started"
echo ""

# Run the audit
echo "🔍 Running NIP-01 smoke tests..."
cd grasp-audit
nix develop -c cargo run -- audit --relay ws://127.0.0.1:9000 --spec nip01-smoke

# Capture exit code
AUDIT_EXIT=$?

# Stop the relay
echo ""
echo "🛑 Stopping relay..."
kill $RELAY_PID 2>/dev/null || true
wait $RELAY_PID 2>/dev/null || true

# Show relay log if there were errors
if [ $AUDIT_EXIT -ne 0 ]; then
    echo ""
    echo "📋 Relay log:"
    cat /tmp/relay.log
fi

exit $AUDIT_EXIT
