#!/bin/bash
set -e

echo "=== Phase 1 Acceptance Tests ==="

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"

if [ ! -x "$TINYBOX" ]; then
    echo "Building tinybox..."
    cargo build
fi

echo -n "Test 1: echo hello... "
OUTPUT=$($TINYBOX run -- echo hello)
if [ "$OUTPUT" = "hello" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 2: exit code 42... "
$TINYBOX run -- sh -c "exit 42" && CODE=0 || CODE=$?
if [ "$CODE" -eq 42 ]; then
    echo "PASS"
else
    echo "FAIL (got: $CODE)"
    exit 1
fi

echo "=== All Phase 1 tests passed ==="
