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
echo "=== Phase 8 API tests passed ==="
