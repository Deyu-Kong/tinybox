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

# P0-3 regression: a normal fork (clone with SIGCHLD only) must still work,
# proving the clone flag-mask rule does not break ordinary process spawning.
cat > "$TEST_DIR/test_fork.c" << 'EOF'
#include <unistd.h>
#include <sys/wait.h>
#include <stdlib.h>
int main(void) {
    pid_t p = fork();
    if (p < 0) _exit(2);
    if (p == 0) _exit(0);
    int st = 0;
    waitpid(p, &st, 0);
    _exit(WIFEXITED(st) && WEXITSTATUS(st) == 0 ? 0 : 3);
}
EOF
gcc -o "$TEST_DIR/test_fork" "$TEST_DIR/test_fork.c"
echo -n "Test 5: normal fork (clone SIGCHLD) still works under seccomp... "
OUTPUT=$($TINYBOX run -- "$TEST_DIR/test_fork" 2>&1) && CODE=0 || CODE=$?
if [ "$CODE" -eq 0 ]; then
    echo "PASS"
else
    echo "FAIL (expected exit 0, got: $CODE)"
    echo "Output: $OUTPUT"
    exit 1
fi

# P0-3 regression: clone(CLONE_NEWUSER) must be killed with SIGSYS (159),
# proving the sandbox cannot create fresh namespaces to sidestep isolation.
cat > "$TEST_DIR/test_clone_newuser.c" << 'EOF'
#define _GNU_SOURCE
#include <sched.h>
#include <signal.h>
#include <sys/wait.h>
#include <stdlib.h>
#include <unistd.h>
static int child_fn(void *a) { (void)a; _exit(0); }
int main(void) {
    char stack[8192];
    int flags = 0x10000000 /* CLONE_NEWUSER */ | SIGCHLD;
    pid_t p = clone(child_fn, stack + sizeof(stack), flags, NULL);
    if (p < 0) _exit(1);      /* seccomp should kill us before here */
    waitpid(p, NULL, 0);
    _exit(0);
}
EOF
gcc -o "$TEST_DIR/test_clone_newuser" "$TEST_DIR/test_clone_newuser.c"
echo -n "Test 6: clone(CLONE_NEWUSER) is blocked by seccomp (P0-3)... "
OUTPUT=$($TINYBOX run -- "$TEST_DIR/test_clone_newuser" 2>&1) && CODE=0 || CODE=$?
if [ "$CODE" -eq 159 ]; then
    echo "PASS (exit 159 = SIGSYS)"
else
    echo "FAIL (expected 159/SIGSYS, got: $CODE)"
    echo "Output: $OUTPUT"
    exit 1
fi

# P0-4 regression: the capability bounding set must be cleared for
# CAP_SYS_ADMIN (bit 21), so setuid execs cannot re-acquire it.
echo -n "Test 7: capability bounding set cleared (P0-4)... "
BND=$($TINYBOX run -- cat /proc/self/status | grep '^CapBnd:' | awk '{print $2}')
if [ $((0x$BND & 0x200000)) -eq 0 ]; then
    echo "PASS (CapBnd=$BND)"
else
    echo "FAIL (CapBnd=$BND has CAP_SYS_ADMIN set)"
    exit 1
fi

echo "=== All Phase 5 tests passed ==="
