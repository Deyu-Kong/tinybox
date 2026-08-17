#!/bin/bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "SKIP: C3 acceptance requires root" >&2
    exit 77
fi

TINYBOX=./target/debug/tinybox
C3_TMP=$(mktemp -d)
C3_PORT=18096
C3_SERVER_PID=""
cleanup() {
    if [ -n "$C3_SERVER_PID" ]; then
        kill "$C3_SERVER_PID" 2>/dev/null || true
    fi
    rm -rf "$C3_TMP"
}
trap cleanup EXIT

echo broker-ok >"$C3_TMP/index.html"
python3 -m http.server "$C3_PORT" --bind 127.0.0.1 --directory "$C3_TMP" \
    >"$C3_TMP/server.log" 2>&1 &
C3_SERVER_PID=$!
sleep 0.2

cat >"$C3_TMP/policy.json" <<EOF
{"version":1,"filesystem":[],"network":[{"host":"localhost","port":$C3_PORT}],"resources":{"memory_bytes":268435456,"cpus":1.0,"pids":50},"phases":[]}
EOF

CLIENT='import socket
s=socket.create_connection(("127.0.0.1",18080))
s.sendall(b"CONNECT localhost:'"$C3_PORT"' HTTP/1.1\r\nHost: localhost\r\n\r\n")
r=s.recv(4096)
assert b"200 Connection Established" in r, r
s.sendall(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
data=b""
while True:
    part=s.recv(4096)
    if not part: break
    data+=part
assert b"broker-ok" in data, data'
"$TINYBOX" run --root / --policy "$C3_TMP/policy.json" -- /usr/bin/python3 -c "$CLIENT"

DIRECT='import socket
try:
    socket.create_connection(("127.0.0.1",'"$C3_PORT"'),timeout=0.3)
except OSError:
    raise SystemExit(0)
raise SystemExit(1)'
"$TINYBOX" run --root / --policy "$C3_TMP/policy.json" -- /usr/bin/python3 -c "$DIRECT"

DENIED='import socket
s=socket.create_connection(("127.0.0.1",18080))
s.sendall(b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n\r\n")
assert b"403 Forbidden" in s.recv(4096)'
"$TINYBOX" run --root / --policy "$C3_TMP/policy.json" -- /usr/bin/python3 -c "$DENIED"

echo "C3 acceptance passed"
