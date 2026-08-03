#!/bin/bash
set -e

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"

echo "=== Phase 12: Volume Mount Tests ==="

echo -n "Test 1: Mount host directory into container... "
mkdir -p /tmp/tinybox-test-volume
echo "hello from host" > /tmp/tinybox-test-volume/test.txt
OUTPUT=$($TINYBOX run --image alpine -v /tmp/tinybox-test-volume:/data -- cat /data/test.txt 2>&1)
if [ "$OUTPUT" = "hello from host" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 2: Read-only volume mount... "
OUTPUT=$($TINYBOX run --image alpine -v /tmp/tinybox-test-volume:/data:ro -- sh -c "echo test > /data/new.txt 2>&1" || true)
if echo "$OUTPUT" | grep -q "Read-only file system"; then
    echo "PASS"
else
    echo "FAIL (expected read-only error, got: $OUTPUT)"
    exit 1
fi

echo -n "Test 3: Multiple volume mounts... "
mkdir -p /tmp/tinybox-test-volume2
echo "second volume" > /tmp/tinybox-test-volume2/data.txt
OUTPUT=$($TINYBOX run --image alpine \
    -v /tmp/tinybox-test-volume:/data1 \
    -v /tmp/tinybox-test-volume2:/data2 \
    -- sh -c "cat /data1/test.txt && cat /data2/data.txt" 2>&1)
if [ "$OUTPUT" = "hello from hostsecond volume" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

rm -rf /tmp/tinybox-test-volume /tmp/tinybox-test-volume2

echo "=== All Phase 12 tests passed ==="
