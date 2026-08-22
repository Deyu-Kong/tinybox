#!/bin/bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "This demo requires root: sudo ./scripts/demo_agent_capabilities.sh" >&2
    exit 77
fi

TINYBOX=${TINYBOX:-./target/debug/tinybox}
DEMO_PAUSE=${DEMO_PAUSE:-1}
DEMO_FIXTURE_PORT=${DEMO_FIXTURE_PORT:-18110}
DEMO_API_PORT=${DEMO_API_PORT:-18111}
DEMO_TMP=$(mktemp -d /var/tmp/tinybox-demo.XXXXXX)
DEMO_FIXTURE_PID=""
DEMO_DAEMON_PID=""

cleanup() {
    [ -z "$DEMO_DAEMON_PID" ] || kill "$DEMO_DAEMON_PID" 2>/dev/null || true
    [ -z "$DEMO_FIXTURE_PID" ] || kill "$DEMO_FIXTURE_PID" 2>/dev/null || true
    rm -rf "$DEMO_TMP"
}
trap cleanup EXIT

step() {
    printf '\n\033[1;36m%s\033[0m\n' "$1"
    sleep "$DEMO_PAUSE"
}

pretty() {
    python3 -m json.tool
}

if [ ! -x "$TINYBOX" ]; then
    echo "Missing $TINYBOX; run cargo build first" >&2
    exit 1
fi
for command in curl python3; do
    command -v "$command" >/dev/null || {
        echo "Missing required command: $command" >&2
        exit 1
    }
done

mkdir "$DEMO_TMP/fixture" "$DEMO_TMP/workspace"
echo "approved package artifact" >"$DEMO_TMP/fixture/package.txt"
echo "customer-secret" >"$DEMO_TMP/secret.txt"
ln -s "$DEMO_TMP/secret.txt" "$DEMO_TMP/workspace/escape"

python3 -m http.server "$DEMO_FIXTURE_PORT" --bind 127.0.0.1 \
    --directory "$DEMO_TMP/fixture" >"$DEMO_TMP/fixture.log" 2>&1 &
DEMO_FIXTURE_PID=$!
"$TINYBOX" daemon --listen "127.0.0.1:$DEMO_API_PORT" \
    >"$DEMO_TMP/daemon.log" 2>&1 &
DEMO_DAEMON_PID=$!
for _ in $(seq 1 40); do
    curl -fsS "http://127.0.0.1:$DEMO_API_PORT/metrics" >/dev/null 2>&1 && break
    sleep 0.1
done

PAYLOAD='import socket,time
def broker(expect):
    s=socket.create_connection(("127.0.0.1",18080))
    s.sendall(b"CONNECT localhost:'"$DEMO_FIXTURE_PORT"' HTTP/1.1\r\n\r\n")
    response=s.recv(1024)
    assert expect in response, response
    s.close()
print("[payload] install: approved dependency access works", flush=True)
broker(b"200")
for port,label in [('"$DEMO_FIXTURE_PORT"',"direct egress"), ('"$DEMO_API_PORT"',"self-grant")]:
    try:
        socket.create_connection(("127.0.0.1",port),timeout=.2)
    except OSError:
        print("[payload] blocked:",label,flush=True)
    else:
        raise SystemExit(label+" unexpectedly succeeded")
time.sleep(4)
broker(b"403")
print("[payload] build: dependency access was revoked", flush=True)'

REQUEST=$(python3 -c 'import json,sys
port=int(sys.argv[2])
ceiling={"memory_bytes":268435456,"cpus":1.0,"pids":50}
print(json.dumps({
  "rootfs":"/",
  "command":["/usr/bin/python3","-c",sys.argv[1]],
  "policy":{
    "version":1,
    "filesystem":[],
    "network":[{"host":"localhost","port":port}],
    "resources":ceiling,
    "phases":[
      {"name":"install","network":[{"host":"localhost","port":port}],"resources":ceiling,"next":["build"]},
      {"name":"build","network":[],"resources":{"memory_bytes":134217728,"cpus":0.5,"pids":20},"next":[]}
    ]
  }
}))' "$PAYLOAD" "$DEMO_FIXTURE_PORT")

step "1/7  User policy starts the Agent in install phase"
CREATE=$(curl -fsS -X POST "http://127.0.0.1:$DEMO_API_PORT/api/sandboxes" \
    -H 'Content-Type: application/json' -d "$REQUEST")
pretty <<<"$CREATE"
DEMO_ID=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<<"$CREATE")

for _ in $(seq 1 40); do
    AUDIT=$(curl -fsS "http://127.0.0.1:$DEMO_API_PORT/api/sandboxes/$DEMO_ID/audit")
    grep -q '"source":"broker","decision":"allow"' <<<"$AUDIT" && break
    sleep 0.1
done

step "2/7  The allowlisted dependency works; direct egress and self-grant do not"
grep '^\[payload\]' "$DEMO_TMP/daemon.log" || true

step "3/7  A forged phase marker is rejected"
curl -sS -w '\nHTTP %{http_code}\n' -X POST \
    "http://127.0.0.1:$DEMO_API_PORT/api/sandboxes/$DEMO_ID/phase" \
    -H 'Content-Type: application/json' \
    -d '{"phase":"forged","expected_generation":0}'

step "4/7  The orchestrator switches install -> build with generation CAS"
SWITCH=$(curl -fsS -X POST \
    "http://127.0.0.1:$DEMO_API_PORT/api/sandboxes/$DEMO_ID/phase" \
    -H 'Content-Type: application/json' \
    -d '{"phase":"build","expected_generation":0}')
pretty <<<"$SWITCH"
printf 'memory.max = %s bytes\n' "$(<"/sys/fs/cgroup/tinybox-$DEMO_ID/memory.max")"
printf 'cpu.max    = %s\n' "$(<"/sys/fs/cgroup/tinybox-$DEMO_ID/cpu.max")"
printf 'pids.max   = %s\n' "$(<"/sys/fs/cgroup/tinybox-$DEMO_ID/pids.max")"

step "5/7  Replaying the old generation is rejected"
curl -sS -w '\nHTTP %{http_code}\n' -X POST \
    "http://127.0.0.1:$DEMO_API_PORT/api/sandboxes/$DEMO_ID/phase" \
    -H 'Content-Type: application/json' \
    -d '{"phase":"build","expected_generation":0}'

for _ in $(seq 1 70); do
    STATUS=$(curl -fsS "http://127.0.0.1:$DEMO_API_PORT/api/sandboxes/$DEMO_ID")
    grep -q '"status":"running"' <<<"$STATUS" || break
    sleep 0.1
done

step "6/7  The same process immediately loses network access; audit explains why"
grep '^\[payload\]' "$DEMO_TMP/daemon.log" || true
curl -fsS "http://127.0.0.1:$DEMO_API_PORT/api/sandboxes/$DEMO_ID/audit" | \
    python3 -c 'import json,sys
for event in json.load(sys.stdin)["events"]:
    if event["capability"] in ("network.connect","phase.transition"):
        print(f"{event['"'"'phase'"'"']:8} {event['"'"'decision'"'"']:5} {event['"'"'capability'"'"']:18} {event['"'"'target'"'"']} — {event['"'"'reason'"'"']}")'

cat >"$DEMO_TMP/host-policy.json" <<EOF
{"version":1,"filesystem":[{"path":"$DEMO_TMP/workspace","access":"read_write"}],"network":[],"resources":{"memory_bytes":268435456,"cpus":1.0,"pids":20},"phases":[]}
EOF

step "7/7  Host Agent may edit its workspace, but cannot read a sibling secret or symlink to it"
"$TINYBOX" agent-host --policy "$DEMO_TMP/host-policy.json" -- \
    /bin/sh -c "echo build-output >'$DEMO_TMP/workspace/result.txt'"
printf 'workspace write: ALLOWED\n'
if "$TINYBOX" agent-host --policy "$DEMO_TMP/host-policy.json" -- \
    /bin/cat "$DEMO_TMP/secret.txt" >/dev/null 2>&1; then
    echo "secret read: UNEXPECTEDLY ALLOWED"
    exit 1
else
    echo "secret read: BLOCKED by Landlock"
fi
if "$TINYBOX" agent-host --policy "$DEMO_TMP/host-policy.json" -- \
    /bin/cat "$DEMO_TMP/workspace/escape" >/dev/null 2>&1; then
    echo "symlink escape: UNEXPECTEDLY ALLOWED"
    exit 1
else
    echo "symlink escape: BLOCKED by Landlock"
fi

printf '\n\033[1;32mDemo complete: declared capability -> kernel enforcement -> dynamic revoke -> audit evidence.\033[0m\n'
