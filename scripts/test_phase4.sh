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

IS_WSL=0
if grep -q "WSL\|Microsoft" /proc/version 2>/dev/null; then
    IS_WSL=1
fi

if [ "$IS_WSL" -eq 1 ]; then
    echo "WARNING: WSL2 detected, cgroup memory limits may not work"
    echo "Test 1: memory limit OOM kill... SKIP (WSL2 limitation)"
else
    echo -n "Test 1: memory limit OOM kill... "
    OUTPUT=$($TINYBOX run --mem-limit 64M -- sh -c "dd if=/dev/zero of=/dev/null bs=1M count=200" 2>&1) && CODE=0 || CODE=$?
    if [ "$CODE" -eq 137 ] || [ "$CODE" -eq 1 ]; then
        echo "PASS (exit code: $CODE)"
    else
        echo "FAIL (expected exit code 137 or 1, got: $CODE)"
        echo "Output: $OUTPUT"
        exit 1
    fi
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

echo "=== All Phase 4 tests passed ==="
