#!/bin/bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "SKIP: C6 acceptance requires root" >&2
    exit 77
fi

TINYBOX=./target/debug/tinybox
C6_TMP=$(mktemp -d /var/tmp/tinybox-c6.XXXXXX)
C6_DAEMON_PID=""
cleanup() {
    [ -z "$C6_DAEMON_PID" ] || kill "$C6_DAEMON_PID" 2>/dev/null || true
    rm -rf "$C6_TMP"
}
trap cleanup EXIT
mkdir "$C6_TMP/workspace"
echo private >"$C6_TMP/secret"
ln -s "$C6_TMP/secret" "$C6_TMP/workspace/escape"
cat >"$C6_TMP/policy.json" <<EOF
{"version":1,"filesystem":[{"path":"$C6_TMP/workspace","access":"read_write"}],"network":[],"resources":{"memory_bytes":268435456,"cpus":1.0,"pids":12},"phases":[]}
EOF

# The wrapper preserves stdout and the payload exit code without accepting policy flags.
OUTPUT=$(TINYBOX_BIN="$TINYBOX" TINYBOX_ROOT=/ TINYBOX_POLICY="$C6_TMP/policy.json" \
    ./scripts/tinybox-agent-tool /bin/sh -c 'printf wrapper-ok')
[ "$OUTPUT" = wrapper-ok ]
if TINYBOX_BIN="$TINYBOX" TINYBOX_ROOT=/ TINYBOX_POLICY="$C6_TMP/policy.json" \
    ./scripts/tinybox-agent-tool /bin/sh -c 'exit 42'; then
    echo "FAIL: wrapper lost payload exit status" >&2
    exit 1
else
    [ "$?" = 42 ]
fi

# Host-side Agent confinement allows the workspace and blocks sibling/symlink escape.
"$TINYBOX" agent-host --policy "$C6_TMP/policy.json" -- \
    /bin/sh -c "echo ok >'$C6_TMP/workspace/output'"
if "$TINYBOX" agent-host --policy "$C6_TMP/policy.json" -- \
    /bin/cat "$C6_TMP/secret" >/dev/null 2>&1; then
    echo "FAIL: host Agent read outside its filesystem ceiling" >&2
    exit 1
fi
if "$TINYBOX" agent-host --policy "$C6_TMP/policy.json" -- \
    /bin/cat "$C6_TMP/workspace/escape" >/dev/null 2>&1; then
    echo "FAIL: host Agent escaped through a symlink" >&2
    exit 1
fi

# The payload cannot reach the host control plane and cannot evade the pids ceiling.
"$TINYBOX" daemon --listen 127.0.0.1:18103 >"$C6_TMP/daemon.log" 2>&1 &
C6_DAEMON_PID=$!
sleep 0.2
NO_CONTROL='import socket
try: socket.create_connection(("127.0.0.1",18103),timeout=.2)
except OSError: raise SystemExit(0)
raise SystemExit(1)'
"$TINYBOX" run --root / --policy "$C6_TMP/policy.json" -- /usr/bin/python3 -c "$NO_CONTROL"
PIDS='import os
children=[]
blocked=False
try:
 for _ in range(50):
  try:
   pid=os.fork()
  except OSError:
   blocked=True; break
  if pid==0: os._exit(0)
  children.append(pid)
finally:
 for pid in children:
  try: os.waitpid(pid,0)
  except ChildProcessError: pass
raise SystemExit(0 if blocked else 1)'
"$TINYBOX" run --root / --policy "$C6_TMP/policy.json" -- /usr/bin/python3 -c "$PIDS"

echo "C6 acceptance passed"
