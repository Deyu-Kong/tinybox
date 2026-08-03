#!/bin/bash
set -e

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"

echo "=== Phase 13: Exec Tests ==="

echo -n "Test 1: Start a long-running container... "
$TINYBOX run --image alpine -- sleep 300 &
CONTAINER_PID=$!
sleep 2
echo "PASS (PID: $CONTAINER_PID)"

echo -n "Test 2: Exec into running container... "
OUTPUT=$($TINYBOX exec --pid $CONTAINER_PID -- echo "hello from exec" 2>&1)
if [ "$OUTPUT" = "hello from exec" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    kill $CONTAINER_PID 2>/dev/null || true
    exit 1
fi

echo -n "Test 3: Exec with multiple commands... "
OUTPUT=$($TINYBOX exec --pid $CONTAINER_PID -- sh -c "echo test1 && echo test2" 2>&1)
if [ "$OUTPUT" = "test1test2" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    kill $CONTAINER_PID 2>/dev/null || true
    exit 1
fi

kill $CONTAINER_PID 2>/dev/null || true
wait $CONTAINER_PID 2>/dev/null || true

echo "=== All Phase 13 tests passed ==="
