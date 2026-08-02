#!/bin/bash
set -e

echo "=== Phase 2 Acceptance Tests ==="

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"

if [ ! -x "$TINYBOX" ]; then
    echo "Building tinybox..."
    cargo build
fi

echo -n "Test 1: ps aux (PID namespace)... "
OUTPUT=$($TINYBOX run -- ps aux)
LINES=$(echo "$OUTPUT" | wc -l)
if [ "$LINES" -le 5 ]; then
    echo "PASS ($LINES lines)"
else
    echo "FAIL (expected <=5 lines, got $LINES)"
    echo "$OUTPUT"
    exit 1
fi

echo -n "Test 2: id (uid=0)... "
OUTPUT=$($TINYBOX run -- id)
if echo "$OUTPUT" | grep -q "uid=0"; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 3: hostname sbox1... "
OUTPUT=$($TINYBOX run --hostname sbox1 -- hostname)
if [ "$OUTPUT" = "sbox1" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo "=== All Phase 2 tests passed ==="
