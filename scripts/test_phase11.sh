#!/bin/bash
set -e

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"

echo "=== Phase 11: Network Bridge Tests ==="

echo -n "Test 1: Container with bridge network has connectivity... "
OUTPUT=$($TINYBOX run --network bridge -- ping -c 1 8.8.8.8 2>&1)
if echo "$OUTPUT" | grep -q "1 packets transmitted, 1 received"; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 2: Container can resolve DNS... "
OUTPUT=$($TINYBOX run --network bridge -- nslookup google.com 2>&1)
if echo "$OUTPUT" | grep -q "Address:"; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 3: Port mapping works... "
# Start a simple HTTP server in the background
$TINYBOX run --network bridge -p 8080:80 -- python3 -m http.server 80 &
SERVER_PID=$!
sleep 2

# Try to connect from host
if curl -s http://localhost:8080 > /dev/null 2>&1; then
    echo "PASS"
else
    echo "FAIL (could not connect to mapped port)"
fi

kill $SERVER_PID 2>/dev/null || true

echo "=== All Phase 11 tests passed ==="
