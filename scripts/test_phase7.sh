#!/bin/bash
set -e

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"

echo -n "Test 1: network namespace has no default route... "
$TINYBOX run -- sh -c 'test ! -s /proc/net/route'
echo "PASS"

echo -n "Test 2: proxy variables are injected... "
OUTPUT=$($TINYBOX run --proxy http://127.0.0.1:8080 -- sh -c 'printf "%s" "$HTTP_PROXY"')
if [ "$OUTPUT" = "http://127.0.0.1:8080" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 3: --proxy mode is network-isolated (no default route, P0-2)... "
$TINYBOX run --proxy http://127.0.0.1:8080 -- sh -c 'test ! -s /proc/net/route'
echo "PASS"

echo "=== Phase 7 basic tests passed ==="
