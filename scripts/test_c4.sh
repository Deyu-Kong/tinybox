#!/bin/bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "SKIP: C4 acceptance requires root" >&2
    exit 77
fi

TINYBOX=./target/debug/tinybox
C4_TMP=$(mktemp -d)
C4_FIXTURE_PORT=18097
C4_API_PORT=18098
C4_SERVER_PID=""
C4_DAEMON_PID=""
cleanup() {
    [ -z "$C4_DAEMON_PID" ] || kill "$C4_DAEMON_PID" 2>/dev/null || true
    [ -z "$C4_SERVER_PID" ] || kill "$C4_SERVER_PID" 2>/dev/null || true
    rm -rf "$C4_TMP"
}
trap cleanup EXIT

echo audit-ok >"$C4_TMP/index.html"
python3 -m http.server "$C4_FIXTURE_PORT" --bind 127.0.0.1 --directory "$C4_TMP" \
    >"$C4_TMP/server.log" 2>&1 &
C4_SERVER_PID=$!
"$TINYBOX" daemon --listen "127.0.0.1:$C4_API_PORT" >"$C4_TMP/daemon.log" 2>&1 &
C4_DAEMON_PID=$!
for _ in $(seq 1 30); do
    curl -fsS "http://127.0.0.1:$C4_API_PORT/metrics" >/dev/null 2>&1 && break
    sleep 0.1
done

CLIENT='import socket
s=socket.create_connection(("127.0.0.1",18080))
s.sendall(b"CONNECT localhost:'"$C4_FIXTURE_PORT"' HTTP/1.1\r\n\r\n")
assert b"200" in s.recv(1024)
s.close()
s=socket.create_connection(("127.0.0.1",18080))
s.sendall(b"CONNECT example.com:443 HTTP/1.1\r\n\r\n")
assert b"403" in s.recv(1024)'
REQUEST=$(python3 -c 'import json,sys; print(json.dumps({"rootfs":"/","command":["/usr/bin/python3","-c",sys.argv[1]],"policy":{"version":1,"filesystem":[],"network":[{"host":"localhost","port":int(sys.argv[2])}],"resources":{"memory_bytes":268435456,"cpus":1.0,"pids":50},"phases":[]}}))' "$CLIENT" "$C4_FIXTURE_PORT")
CREATE=$(curl -fsS -X POST "http://127.0.0.1:$C4_API_PORT/api/sandboxes" \
    -H 'Content-Type: application/json' -d "$REQUEST")
C4_ID=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<<"$CREATE")
for _ in $(seq 1 50); do
    STATUS=$(curl -fsS "http://127.0.0.1:$C4_API_PORT/api/sandboxes/$C4_ID")
    grep -q '"status":"running"' <<<"$STATUS" || break
    sleep 0.1
done
grep -q '"status":"completed"' <<<"$STATUS"

AUDIT=$(curl -fsS "http://127.0.0.1:$C4_API_PORT/api/sandboxes/$C4_ID/audit")
grep -q '"source":"broker","decision":"allow"' <<<"$AUDIT"
grep -q '"source":"broker","decision":"deny"' <<<"$AUDIT"
grep -q '"capability":"sandbox.setup"' <<<"$AUDIT"
grep -q '"dropped_events":0' <<<"$AUDIT"
SUMMARY=$(curl -fsS "http://127.0.0.1:$C4_API_PORT/api/sandboxes/$C4_ID/audit/summary")
grep -q '"allow"' <<<"$SUMMARY"
grep -q '"deny"' <<<"$SUMMARY"

echo "C4 acceptance passed"
