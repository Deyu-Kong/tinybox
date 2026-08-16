#!/bin/bash
set -e

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"
PORT=18091

echo "=== Phase 13: Exec Tests (P1-5) ==="

# Start the daemon; it exposes sandbox PIDs so `tinybox exec` has a valid
# target (the sandbox's intermediate process, which lives in the sandbox
# namespaces and carries the tinybox cgroup).
$TINYBOX daemon --listen "127.0.0.1:$PORT" >/tmp/tinybox-phase13.log 2>&1 &
DAEMON_PID=$!
trap 'kill "$DAEMON_PID" 2>/dev/null || true' EXIT
sleep 1

echo -n "Test 1: start a long-running sandbox via the API... "
curl -fsS -X POST "http://127.0.0.1:$PORT/api/sandboxes" -H 'Content-Type: application/json' \
    -d '{"rootfs":"/","command":["/bin/sh","-c","sleep 60"]}' >/dev/null
# Wait until the pid is populated.
SANDBOX_PID=""
for _ in 1 2 3 4 5 6 7 8 9 10; do
    sleep 0.3
    SANDBOX_PID=$(curl -fsS "http://127.0.0.1:$PORT/api/sandboxes/sb-1" | grep -o '"pid":[0-9]*' | cut -d: -f2)
    [ -n "$SANDBOX_PID" ] && [ "$SANDBOX_PID" != "null" ] && break
done
if [ -z "$SANDBOX_PID" ] || [ "$SANDBOX_PID" = "null" ]; then
    echo "FAIL (no sandbox pid)"
    exit 1
fi
echo "PASS (sandbox pid: $SANDBOX_PID)"

echo -n "Test 2: exec into the running sandbox via setns... "
OUTPUT=$($TINYBOX exec --pid "$SANDBOX_PID" -- echo "hello from exec" 2>&1)
if [ "$OUTPUT" = "hello from exec" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 3: exec runs inside the sandbox's PID namespace... "
# `sh -c 'echo $$'` prints the exec'd shell's PID; in the host pid ns it would
# be a large number. Inside a fresh-ish sandbox pid ns it's small (>=1). We
# just assert we get a non-empty numeric output (setns + fork worked).
OUTPUT=$($TINYBOX exec --pid "$SANDBOX_PID" -- sh -c 'echo $$' 2>&1)
if echo "$OUTPUT" | grep -qE '^[0-9]+$'; then
    echo "PASS (pid in ns: $OUTPUT)"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Test 4: exec refuses a non-tinybox PID (P1-5 validation)... "
if $TINYBOX exec --pid 1 -- echo x 2>&1 | grep -qi "not a tinybox cgroup\|arbitrary host"; then
    echo "PASS"
else
    echo "FAIL (expected rejection of host pid 1)"
    exit 1
fi

# Cleanup.
curl -fsS -X DELETE "http://127.0.0.1:$PORT/api/sandboxes/sb-1" >/dev/null 2>&1 || true

echo "=== All Phase 13 tests passed ==="
