#!/bin/bash
set -e

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"

echo "=== Phase 10: Docker Registry Pull Tests ==="

echo -n "Test 1: Pull alpine image... "
if $TINYBOX image pull alpine --alias alpine-pulled 2>&1 | grep -q "pulled:"; then
    echo "PASS"
else
    echo "FAIL (network issue or registry unavailable)"
    exit 1
fi

echo -n "Test 2: Run container from pulled image... "
OUTPUT=$($TINYBOX run --image alpine-pulled -- /bin/sh -c "echo hello-from-pulled")
if [ "$OUTPUT" = "hello-from-pulled" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 3: List shows pulled image... "
if $TINYBOX image list | grep -q "alpine-pulled"; then
    echo "PASS"
else
    echo "FAIL"
    exit 1
fi

echo -n "Test 4: Remove pulled image... "
$TINYBOX image remove alpine-pulled
if ! $TINYBOX image list | grep -q "alpine-pulled"; then
    echo "PASS"
else
    echo "FAIL"
    exit 1
fi

echo "=== All Phase 10 tests passed ==="
