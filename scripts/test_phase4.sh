#!/bin/bash
set -e

echo "=== Phase 4 Acceptance Tests ==="

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"

if [ ! -x "$TINYBOX" ]; then
    echo "Building tinybox..."
    cargo build
fi

echo -n "Test 1: memory limit OOM kill... "
OUTPUT=$($TINYBOX run --mem-limit 64M -- python3 -c "a = bytearray(200*1024*1024)" 2>&1) && CODE=0 || CODE=$?
if [ "$CODE" -eq 137 ] || [ "$CODE" -eq 1 ]; then
    echo "PASS (exit code: $CODE)"
else
    echo "FAIL (expected exit code 137 or 1, got: $CODE)"
    echo "Output: $OUTPUT"
    exit 1
fi

echo -n "Test 2: memory limit normal operation... "
OUTPUT=$($TINYBOX run --mem-limit 256M -- echo hello)
if [ "$OUTPUT" = "hello" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 3: invalid memory limit... "
if $TINYBOX run --mem-limit invalid -- echo test 2>&1 | grep -q "invalid"; then
    echo "PASS"
else
    echo "FAIL"
    exit 1
fi

echo -n "Test 4: cpu limit normal operation... "
OUTPUT=$($TINYBOX run --cpu-limit 50 -- echo hello)
if [ "$OUTPUT" = "hello" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 5: invalid cpu limit... "
if $TINYBOX run --cpu-limit 0 -- echo test 2>&1; then
    echo "FAIL (should have rejected cpu-limit 0)"
    exit 1
else
    echo "PASS"
fi

echo "=== All Phase 4 tests passed ==="
