#!/bin/bash
set -e

echo "=== Phase 5 Acceptance Tests ==="

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"

if [ ! -x "$TINYBOX" ]; then
    echo "Building tinybox..."
    cargo build
fi

TEST_DIR=$(mktemp -d)
trap "rm -rf $TEST_DIR" EXIT

cat > "$TEST_DIR/test_reboot.c" << 'EOF'
#include <unistd.h>
#include <sys/reboot.h>

int main() {
    reboot(RB_AUTOBOOT);
    return 0;
}
EOF
gcc -o "$TEST_DIR/test_reboot" "$TEST_DIR/test_reboot.c"

echo -n "Test 1: seccomp blocks reboot syscall... "
OUTPUT=$($TINYBOX run -- "$TEST_DIR/test_reboot" 2>&1) && CODE=0 || CODE=$?
if [ "$CODE" -eq 159 ]; then
    echo "PASS (exit code: $CODE, SIGSYS)"
else
    echo "FAIL (expected exit code 159, got: $CODE)"
    echo "Output: $OUTPUT"
    exit 1
fi

echo -n "Test 2: seccomp blocks mount... "
OUTPUT=$($TINYBOX run -- mount -t tmpfs none /tmp 2>&1) && CODE=0 || CODE=$?
if [ "$CODE" -ne 0 ]; then
    echo "PASS (exit code: $CODE)"
else
    echo "FAIL (expected non-zero exit code, got: $CODE)"
    echo "Output: $OUTPUT"
    exit 1
fi

echo -n "Test 3: --dangerous allows mount... "
OUTPUT=$($TINYBOX run --dangerous -- mount -t tmpfs none /tmp 2>&1) && CODE=0 || CODE=$?
if [ "$CODE" -eq 0 ]; then
    echo "PASS"
    umount /tmp 2>/dev/null || true
else
    echo "FAIL (expected exit code 0, got: $CODE)"
    echo "Output: $OUTPUT"
    exit 1
fi

echo -n "Test 4: seccomp allows echo... "
OUTPUT=$($TINYBOX run -- echo hello)
if [ "$OUTPUT" = "hello" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo "=== All Phase 5 tests passed ==="
