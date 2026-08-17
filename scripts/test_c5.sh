#!/bin/bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "SKIP: C5 acceptance requires root" >&2
    exit 77
fi

TINYBOX=./target/debug/tinybox
C5_TMP=$(mktemp -d)
C5_FIXTURE_PORT=18099
C5_API_PORT=18102
C5_SERVER_PID=""
C5_DAEMON_PID=""
cleanup() {
    [ -z "$C5_DAEMON_PID" ] || kill "$C5_DAEMON_PID" 2>/dev/null || true
    [ -z "$C5_SERVER_PID" ] || kill "$C5_SERVER_PID" 2>/dev/null || true
    rm -rf "$C5_TMP"
}
trap cleanup EXIT

python3 -m http.server "$C5_FIXTURE_PORT" --bind 127.0.0.1 --directory "$C5_TMP" \
    >"$C5_TMP/server.log" 2>&1 &
C5_SERVER_PID=$!
"$TINYBOX" daemon --listen "127.0.0.1:$C5_API_PORT" >"$C5_TMP/daemon.log" 2>&1 &
C5_DAEMON_PID=$!
for _ in $(seq 1 30); do
    curl -fsS "http://127.0.0.1:$C5_API_PORT/metrics" >/dev/null 2>&1 && break
    sleep 0.1
done

CLIENT='import socket,time
def connect(expect):
 s=socket.create_connection(("127.0.0.1",18080))
 s.sendall(b"CONNECT localhost:'"$C5_FIXTURE_PORT"' HTTP/1.1\r\n\r\n")
 data=s.recv(1024)
 assert expect in data, data
 s.close()
connect(b"200")
time.sleep(4)
connect(b"403")'
REQUEST=$(python3 -c 'import json,sys; port=int(sys.argv[2]); resources={"memory_bytes":268435456,"cpus":1.0,"pids":50}; print(json.dumps({"rootfs":"/","command":["/usr/bin/python3","-c",sys.argv[1]],"policy":{"version":1,"filesystem":[],"network":[{"host":"localhost","port":port}],"resources":resources,"phases":[{"name":"install","network":[{"host":"localhost","port":port}],"resources":resources,"next":["build"]},{"name":"build","network":[],"resources":{"memory_bytes":134217728,"cpus":0.5,"pids":20},"next":[]}]}}))' "$CLIENT" "$C5_FIXTURE_PORT")
CREATE=$(curl -fsS -X POST "http://127.0.0.1:$C5_API_PORT/api/sandboxes" \
    -H 'Content-Type: application/json' -d "$REQUEST")
C5_ID=$(python3 -c 'import json,sys; value=json.load(sys.stdin); assert value["phase"]=="install" and value["generation"]==0; print(value["id"])' <<<"$CREATE")

for _ in $(seq 1 30); do
    AUDIT=$(curl -fsS "http://127.0.0.1:$C5_API_PORT/api/sandboxes/$C5_ID/audit")
    grep -q '"source":"broker","decision":"allow"' <<<"$AUDIT" && break
    sleep 0.1
done
grep -q '"source":"broker","decision":"allow"' <<<"$AUDIT"

CODE=$(curl -sS -o "$C5_TMP/forged.json" -w '%{http_code}' -X POST \
    "http://127.0.0.1:$C5_API_PORT/api/sandboxes/$C5_ID/phase" \
    -H 'Content-Type: application/json' -d '{"phase":"forged","expected_generation":0}')
[ "$CODE" = 409 ]
SWITCH=$(curl -fsS -X POST "http://127.0.0.1:$C5_API_PORT/api/sandboxes/$C5_ID/phase" \
    -H 'Content-Type: application/json' -d '{"phase":"build","expected_generation":0}')
grep -q '"phase":"build"' <<<"$SWITCH"
grep -q '"generation":1' <<<"$SWITCH"
[ "$(cat "/sys/fs/cgroup/tinybox-$C5_ID/memory.max")" = 134217728 ]

CODE=$(curl -sS -o "$C5_TMP/replay.json" -w '%{http_code}' -X POST \
    "http://127.0.0.1:$C5_API_PORT/api/sandboxes/$C5_ID/phase" \
    -H 'Content-Type: application/json' -d '{"phase":"build","expected_generation":0}')
[ "$CODE" = 409 ]

for _ in $(seq 1 60); do
    STATUS=$(curl -fsS "http://127.0.0.1:$C5_API_PORT/api/sandboxes/$C5_ID")
    grep -q '"status":"running"' <<<"$STATUS" || break
    sleep 0.1
done
grep -q '"status":"completed"' <<<"$STATUS"
AUDIT=$(curl -fsS "http://127.0.0.1:$C5_API_PORT/api/sandboxes/$C5_ID/audit")
grep -q '"phase":"build","source":"broker","decision":"deny"' <<<"$AUDIT"
grep -q '"capability":"phase.transition"' <<<"$AUDIT"

echo "C5 acceptance passed"
