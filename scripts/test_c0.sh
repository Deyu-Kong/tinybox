#!/bin/bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "SKIP: C0 acceptance requires root" >&2
    exit 77
fi

TINYBOX=./target/debug/tinybox
C0_TMP=$(mktemp -d)
C0_PORT=18094
C0_DAEMON_PID=""
cleanup() {
    if [ -n "$C0_DAEMON_PID" ]; then
        kill "$C0_DAEMON_PID" 2>/dev/null || true
    fi
    rm -rf "$C0_TMP"
}
trap cleanup EXIT

mkdir -p "$C0_TMP/bundle/rootfs" "$C0_TMP/volume"

cat >"$C0_TMP/bundle/config.json" <<'EOF'
{"process":{"args":["true"]},"root":{"path":"rootfs"},"linux":{"namespaces":[{"type":"pid"}]}}
EOF
if "$TINYBOX" run --oci "$C0_TMP/bundle" 2>"$C0_TMP/no-mount.err"; then
    echo "FAIL: OCI rootfs without mount namespace was accepted" >&2
    exit 1
fi
grep -q "private mount namespace is required" "$C0_TMP/no-mount.err"

cat >"$C0_TMP/bundle/config.json" <<'EOF'
{"process":{"args":["true"]},"root":{"path":"rootfs"},"linux":{"namespaces":[{"type":"user"},{"type":"mount"}]}}
EOF
if "$TINYBOX" run --oci "$C0_TMP/bundle" 2>"$C0_TMP/user.err"; then
    echo "FAIL: unsupported user namespace was accepted" >&2
    exit 1
fi
grep -q "user namespace is unsupported" "$C0_TMP/user.err"

"$TINYBOX" daemon --listen "127.0.0.1:$C0_PORT" >"$C0_TMP/daemon.log" 2>&1 &
C0_DAEMON_PID=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do
    curl -fsS "http://127.0.0.1:$C0_PORT/metrics" >/dev/null 2>&1 && break
    sleep 0.1
done

curl -fsS -X POST "http://127.0.0.1:$C0_PORT/api/sandboxes" \
    -H 'Content-Type: application/json' \
    -d '{"rootfs":"/","command":["/definitely/missing"]}' >/dev/null
sleep 0.3
curl -fsS "http://127.0.0.1:$C0_PORT/api/sandboxes/sb-1" \
    | grep -q '"status":"setup_failed"'

curl -fsS -X POST "http://127.0.0.1:$C0_PORT/api/sandboxes" \
    -H 'Content-Type: application/json' \
    -d '{"rootfs":"/","command":["sh","-c","exit 1"]}' >/dev/null
sleep 0.3
SB2=$(curl -fsS "http://127.0.0.1:$C0_PORT/api/sandboxes/sb-2")
echo "$SB2" | grep -q '"status":"completed"'
echo "$SB2" | grep -q '"exit_code":1'

echo original >"$C0_TMP/volume/original"
if "$TINYBOX" run --root / -v "$C0_TMP/volume:/data:ro" -- \
    sh -c 'echo changed > /data/original' 2>"$C0_TMP/readonly.err"; then
    echo "FAIL: read-only bind volume accepted a write" >&2
    exit 1
fi
grep -q '^original$' "$C0_TMP/volume/original"

echo "C0 acceptance passed"
