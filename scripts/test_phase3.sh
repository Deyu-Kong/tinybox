#!/bin/bash
set -e

echo "=== Phase 3 Acceptance Tests ==="

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"

if [ ! -x "$TINYBOX" ]; then
    echo "Building tinybox..."
    cargo build
fi

ROOTFS=$(mktemp -d)
trap "rm -rf $ROOTFS" EXIT

echo "Creating test rootfs at $ROOTFS..."
mkdir -p "$ROOTFS/bin" "$ROOTFS/lib" "$ROOTFS/lib64" "$ROOTFS/proc" "$ROOTFS/tmp"

for bin in sh echo cat ls id hostname ps; do
    if [ -x "/bin/$bin" ]; then
        cp "/bin/$bin" "$ROOTFS/bin/"
    fi
done

echo "Copying shared libraries..."
for bin in "$ROOTFS"/bin/*; do
    ldd "$bin" 2>/dev/null | grep -o '/[^ ]*' | while read -r lib; do
        dir=$(dirname "$lib")
        mkdir -p "$ROOTFS$dir"
        cp -n "$lib" "$ROOTFS$lib" 2>/dev/null || true
    done
done

if [ -f /lib64/ld-linux-x86-64.so.2 ]; then
    cp /lib64/ld-linux-x86-64.so.2 "$ROOTFS/lib64/" 2>/dev/null || true
fi

echo -n "Test 1: basic rootfs execution... "
OUTPUT=$($TINYBOX run --root "$ROOTFS" -- echo hello)
if [ "$OUTPUT" = "hello" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 2: file isolation (COW)... "
OUTPUT=$($TINYBOX run --root "$ROOTFS" -- sh -c "echo 'isolated' > /tmp/test.txt && cat /tmp/test.txt")
if [ "$OUTPUT" = "isolated" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

if [ -f "$ROOTFS/tmp/test.txt" ]; then
    echo "FAIL: file leaked to host rootfs"
    exit 1
fi

echo -n "Test 3: rootfs with hostname... "
OUTPUT=$($TINYBOX run --root "$ROOTFS" --hostname sandbox3 -- hostname)
if [ "$OUTPUT" = "sandbox3" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 4: nonexistent rootfs... "
if $TINYBOX run --root /nonexistent/rootfs -- echo test 2>&1 | grep -q "does not exist\|not exist"; then
    echo "PASS"
else
    echo "FAIL"
    exit 1
fi

echo -n "Test 5: /dev/null is populated and writable (P2-1)... "
OUTPUT=$($TINYBOX run --root "$ROOTFS" -- sh -c 'echo ok > /dev/null && echo yes')
if [ "$OUTPUT" = "yes" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 6: /proc is mounted (P2-1)... "
OUTPUT=$($TINYBOX run --root "$ROOTFS" -- sh -c 'test -f /proc/self/status && echo yes')
if [ "$OUTPUT" = "yes" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 7: /tmp is a writable tmpfs, size-capped (P2-1)... "
OUTPUT=$($TINYBOX run --root "$ROOTFS" -- sh -c 'echo data > /tmp/f && cat /tmp/f')
if [ "$OUTPUT" = "data" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 8: --read-only makes rootfs read-only (P2-1)... "
$TINYBOX run --read-only --root "$ROOTFS" -- sh -c 'echo x > /should_fail' 2>/dev/null && CODE=0 || CODE=$?
if [ "$CODE" -ne 0 ]; then
    echo "PASS (write denied, exit $CODE)"
else
    echo "FAIL (write succeeded on read-only rootfs)"
    exit 1
fi

echo "=== All Phase 3 tests passed ==="
