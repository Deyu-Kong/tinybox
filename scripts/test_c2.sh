#!/bin/bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "SKIP: C2 acceptance requires root" >&2
    exit 77
fi

TINYBOX=./target/debug/tinybox
C2_TMP=$(mktemp -d)
cleanup() {
    rm -rf "$C2_TMP"
}
trap cleanup EXIT

mkdir -p "$C2_TMP/workspace"
echo input >"$C2_TMP/workspace/input.txt"
ln -s /etc/passwd "$C2_TMP/workspace/escape"

cat >"$C2_TMP/read-write.json" <<'EOF'
{"version":1,"filesystem":[{"path":"/workspace","access":"read_write"}],"network":[],"resources":{"memory_bytes":268435456,"cpus":1.0,"pids":50},"phases":[]}
EOF
"$TINYBOX" run --root / --policy "$C2_TMP/read-write.json" \
    -v "$C2_TMP/workspace:/workspace" -- \
    sh -c 'test "$(cat /workspace/input.txt)" = input && echo output > /workspace/output.txt'
grep -q '^output$' "$C2_TMP/workspace/output.txt"

if "$TINYBOX" run --root / --policy "$C2_TMP/read-write.json" \
    -v "$C2_TMP/workspace:/workspace" -- cat /etc/passwd >/dev/null 2>&1; then
    echo "FAIL: Landlock allowed undeclared /etc/passwd" >&2
    exit 1
fi
if "$TINYBOX" run --root / --policy "$C2_TMP/read-write.json" \
    -v "$C2_TMP/workspace:/workspace" -- cat /workspace/escape >/dev/null 2>&1; then
    echo "FAIL: Landlock allowed symlink escape" >&2
    exit 1
fi

cat >"$C2_TMP/read-only.json" <<'EOF'
{"version":1,"filesystem":[{"path":"/workspace","access":"read"}],"network":[],"resources":{"memory_bytes":268435456,"cpus":1.0,"pids":50},"phases":[]}
EOF
if "$TINYBOX" run --root / --policy "$C2_TMP/read-only.json" \
    -v "$C2_TMP/workspace:/workspace" -- \
    sh -c 'echo forbidden > /workspace/forbidden' >/dev/null 2>&1; then
    echo "FAIL: Landlock read-only rule allowed a write" >&2
    exit 1
fi

echo "C2 acceptance passed"
