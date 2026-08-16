#!/bin/bash
set -e

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"
PORT=18081
$TINYBOX daemon --listen "127.0.0.1:$PORT" >/tmp/tinybox-phase8.log 2>&1 &
PID=$!
trap 'kill "$PID" 2>/dev/null || true' EXIT
sleep 1

curl -fsS "http://127.0.0.1:$PORT/metrics" | grep -q 'sandboxes_total'
CREATE=$(curl -fsS -X POST "http://127.0.0.1:$PORT/api/sandboxes" -H 'Content-Type: application/json' -d '{"rootfs":"/","command":["/bin/sh","-c","exit 0"]}')
echo "$CREATE" | grep -q 'sb-'
sleep 1
curl -fsS "http://127.0.0.1:$PORT/api/sandboxes" | grep -q 'completed'
curl -fsS "http://127.0.0.1:$PORT/metrics" | grep -q 'sandboxes_completed'

# P1-4: disabling seccomp/caps remotely must be rejected (HTTP 400).
echo -n "Test: remote dangerous:true rejected (P1-4)... "
DANGER_CODE=$(curl -s -o /tmp/tinybox-danger.json -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/api/sandboxes" -H 'Content-Type: application/json' -d '{"rootfs":"/","command":["echo"],"dangerous":true}')
if [ "$DANGER_CODE" = "400" ]; then
    echo "PASS (400)"
else
    echo "FAIL (expected 400, got $DANGER_CODE)"
    exit 1
fi

# P1-3: a failing sandbox must land in "failed" status and surface in
# /metrics as tinybox_sandboxes_failed (previously miscounted as completed).
curl -fsS -X POST "http://127.0.0.1:$PORT/api/sandboxes" -H 'Content-Type: application/json' -d '{"rootfs":"/nonexistent/rootfs","command":["/bin/sh","-c","true"]}' >/dev/null
sleep 1
echo -n "Test: failing sandbox counted as failed (P1-3)... "
if curl -fsS "http://127.0.0.1:$PORT/api/sandboxes" | grep -q '"failed"'; then
    echo "PASS (status=failed)"
else
    echo "FAIL (no failed status found)"
    exit 1
fi
echo -n "Test: /metrics exposes sandboxes_failed (P1-3)... "
METRICS=$(curl -fsS "http://127.0.0.1:$PORT/metrics")
if echo "$METRICS" | grep -q 'sandboxes_failed' && ! echo "$METRICS" | grep -q 'sandboxes_failed 0'; then
    echo "PASS"
else
    echo "FAIL (sandboxes_failed missing or zero)"
    exit 1
fi

echo "=== Phase 8 API tests passed ==="
