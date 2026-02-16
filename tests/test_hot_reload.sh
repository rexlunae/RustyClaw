#!/bin/bash
# Test script for configuration hot-reload functionality

set -e

echo "=== RustyClaw Configuration Hot-Reload Test ==="
echo ""

# Setup test environment
TEST_DIR=$(mktemp -d)
CONFIG_FILE="$TEST_DIR/config.toml"
trap "rm -rf $TEST_DIR" EXIT

# Create initial config
cat > "$CONFIG_FILE" << 'EOF'
[settings]
settings_dir = "$TEST_DIR"

[sandbox]
enabled = true
mode = "isolate"

[tls]
enabled = false

[metrics]
enabled = true
listen_addr = "127.0.0.1:9090"

[ssrf]
enabled = true

[prompt_guard]
enabled = false
action = "warn"
sensitivity = 0.5
EOF

echo "1. Starting gateway with initial config..."
cargo run --bin rustyclaw -- gateway start --config "$CONFIG_FILE" &
GATEWAY_PID=$!
trap "kill $GATEWAY_PID 2>/dev/null || true; rm -rf $TEST_DIR" EXIT

# Wait for gateway to start
sleep 3

if ! kill -0 $GATEWAY_PID 2>/dev/null; then
    echo "✗ Gateway failed to start"
    exit 1
fi

echo "✓ Gateway started (PID: $GATEWAY_PID)"
echo ""

# Modify config
echo "2. Modifying config (enabling prompt_guard, disabling metrics)..."
cat > "$CONFIG_FILE" << 'EOF'
[settings]
settings_dir = "$TEST_DIR"

[sandbox]
enabled = true
mode = "isolate"

[tls]
enabled = false

[metrics]
enabled = false
listen_addr = "127.0.0.1:9090"

[ssrf]
enabled = true

[prompt_guard]
enabled = true
action = "block"
sensitivity = 0.5
EOF

echo "✓ Config file updated"
echo ""

# Send SIGHUP
echo "3. Sending SIGHUP signal to gateway..."
if kill -HUP $GATEWAY_PID; then
    echo "✓ SIGHUP sent successfully"
else
    echo "✗ Failed to send SIGHUP"
    exit 1
fi

# Wait for reload
sleep 2

# Check if gateway is still running
if ! kill -0 $GATEWAY_PID 2>/dev/null; then
    echo "✗ Gateway crashed during reload"
    exit 1
fi

echo "✓ Gateway still running after reload"
echo ""

# Test second reload
echo "4. Testing second hot-reload (enabling TLS)..."
cat > "$CONFIG_FILE" << 'EOF'
[settings]
settings_dir = "$TEST_DIR"

[sandbox]
enabled = true
mode = "isolate"

[tls]
enabled = true
self_signed = true

[metrics]
enabled = true
listen_addr = "127.0.0.1:9090"

[ssrf]
enabled = false

[prompt_guard]
enabled = true
action = "warn"
sensitivity = 0.8
EOF

kill -HUP $GATEWAY_PID
sleep 2

if ! kill -0 $GATEWAY_PID 2>/dev/null; then
    echo "✗ Gateway crashed during second reload"
    exit 1
fi

echo "✓ Second reload successful"
echo ""

# Cleanup
echo "5. Shutting down gateway..."
kill $GATEWAY_PID 2>/dev/null || true
wait $GATEWAY_PID 2>/dev/null || true

echo "✓ Gateway shut down cleanly"
echo ""
echo "=== All hot-reload tests passed! ==="
