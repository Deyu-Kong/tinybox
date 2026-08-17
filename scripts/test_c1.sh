#!/bin/bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "SKIP: C1 acceptance requires root" >&2
    exit 77
fi

TINYBOX=./target/debug/tinybox
C1_TMP=$(mktemp -d)
C1_PORT=18095
C1_DAEMON_PID=""
cleanup() {
    if [ -n "$C1_DAEMON_PID" ]; then
        kill "$C1_DAEMON_PID" 2>/dev/null || true
    fi
    rm -rf "$C1_TMP"
}
trap cleanup EXIT

cat >"$C1_TMP/offline.json" <<'EOF'
{"version":1,"filesystem":[],"network":[],"resources":{"memory_bytes":268435456,"cpus":1.0,"pids":50},"phases":[]}
EOF

"$TINYBOX" run --policy "$C1_TMP/offline.json" -- true 2>"$C1_TMP/policy.err"
grep -q '^tinybox policy: sha256:' "$C1_TMP/policy.err"

if "$TINYBOX" run --policy "$C1_TMP/offline.json" --memory 512m -- true \
    2>"$C1_TMP/ceiling.err"; then
    echo "FAIL: resource request exceeded policy ceiling" >&2
    exit 1
fi
grep -q 'exceeds the policy ceiling' "$C1_TMP/ceiling.err"

cat >"$C1_TMP/network.json" <<'EOF'
{"version":1,"filesystem":[],"network":[{"host":"example.com","port":443}],"resources":{"memory_bytes":268435456,"cpus":1.0,"pids":50},"phases":[]}
EOF
"$TINYBOX" run --policy "$C1_TMP/network.json" -- true 2>"$C1_TMP/network.err"
grep -q '^tinybox policy: sha256:' "$C1_TMP/network.err"

"$TINYBOX" daemon --listen "127.0.0.1:$C1_PORT" >"$C1_TMP/daemon.log" 2>&1 &
C1_DAEMON_PID=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do
    curl -fsS "http://127.0.0.1:$C1_PORT/metrics" >/dev/null 2>&1 && break
    sleep 0.1
done

CREATE=$(curl -fsS -X POST "http://127.0.0.1:$C1_PORT/api/sandboxes" \
    -H 'Content-Type: application/json' \
    -d '{"rootfs":"/","command":["true"],"policy":{"version":1,"filesystem":[],"network":[],"resources":{"memory_bytes":268435456,"cpus":1.0,"pids":50},"phases":[]}}')
echo "$CREATE" | grep -q '"policy_hash":"sha256:'

echo "C1 acceptance passed"
