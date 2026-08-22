#!/bin/bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "demo requires root" >&2
    exit 77
fi

ITERATIONS=${1:-10}
TINYBOX=${TINYBOX:-./target/release/tinybox}
ADDRESS=${TINYBOX_DEMO_ADDRESS:-127.0.0.1:18120}
DEMO_ROOT=$(mktemp -d /var/tmp/tinybox-local-agent-demo.XXXXXX)
DAEMON_PID=
TASK_ID=
cleanup() {
    [ -z "$TASK_ID" ] || "$TINYBOX" agent destroy "$TASK_ID" >/dev/null 2>&1 || true
    [ -z "$DAEMON_PID" ] || kill "$DAEMON_PID" 2>/dev/null || true
    find "$DEMO_ROOT" -depth -delete 2>/dev/null || true
}
trap cleanup EXIT

if [ ! -x "$TINYBOX" ]; then
    echo "missing $TINYBOX; run cargo build --release" >&2
    exit 1
fi
TINYBOX=$(readlink -f "$TINYBOX")
for command in curl python3 date; do
    command -v "$command" >/dev/null || { echo "missing command: $command" >&2; exit 1; }
done

mkdir -p "$DEMO_ROOT/workspace" "$DEMO_ROOT/samples"
cat >"$DEMO_ROOT/workspace/build.sh" <<'EOF'
#!/bin/sh
set -eu
printf 'tinybox-demo-artifact\n' > artifact.txt
EOF
cat >"$DEMO_ROOT/workspace/test.sh" <<'EOF'
#!/bin/sh
set -eu
test "$(cat artifact.txt)" = tinybox-demo-artifact
printf 'tests-ok\n'
EOF
chmod +x "$DEMO_ROOT/workspace/build.sh" "$DEMO_ROOT/workspace/test.sh"

"$TINYBOX" daemon --listen "$ADDRESS" >"$DEMO_ROOT/daemon.log" 2>&1 &
DAEMON_PID=$!
for _ in $(seq 1 100); do
    curl -fsS "http://$ADDRESS/metrics" >/dev/null 2>&1 && break
    sleep 0.02
done
curl -fsS "http://$ADDRESS/metrics" >/dev/null

measure() {
    local output=$1
    shift
    : >"$output"
    for _ in $(seq 1 "$ITERATIONS"); do
        started=$(date +%s%N)
        "$@" >/dev/null
        finished=$(date +%s%N)
        awk -v start="$started" -v finish="$finished" \
            'BEGIN {printf "%.3f\n", (finish-start)/1000000}' >>"$output"
    done
}

cd "$DEMO_ROOT/workspace"
./build.sh
./test.sh >/dev/null
measure "$DEMO_ROOT/samples/bare.ms" /bin/sh -c './build.sh && ./test.sh'

measure "$DEMO_ROOT/samples/cold.ms" "$TINYBOX" agent run "$DEMO_ROOT/workspace" \
    --daemon "$ADDRESS" -- /bin/sh -c './build.sh && ./test.sh'

TASK_ID=$("$TINYBOX" agent run "$DEMO_ROOT/workspace" --daemon "$ADDRESS" --detach)
measure "$DEMO_ROOT/samples/warm.ms" "$TINYBOX" agent exec "$TASK_ID" -- \
    /bin/sh -c './build.sh && ./test.sh'

STATE_BYTES_BEFORE=$(du -sb "/var/lib/tinybox/tasks/$TASK_ID" | awk '{print $1}')
"$TINYBOX" agent exec "$TASK_ID" -- /bin/sh -c \
    'dd if=/dev/zero of="$XDG_CACHE_HOME/demo-cache" bs=1024 count=64 status=none'
STATE_BYTES_AFTER=$(du -sb "/var/lib/tinybox/tasks/$TASK_ID" | awk '{print $1}')

set +e
"$TINYBOX" agent exec "$TASK_ID" --timeout-ms 100 -- /bin/sh -c 'sleep 10' >/dev/null 2>&1
TIMEOUT_CODE=$?
set -e
[ "$TIMEOUT_CODE" -eq 124 ]
"$TINYBOX" agent exec "$TASK_ID" -- /bin/sh -c 'sleep 30 & printf started' >/dev/null
if find "/sys/fs/cgroup/tinybox-$TASK_ID" -mindepth 1 -maxdepth 1 -type d | grep -q .; then
    echo "exec cgroup remained after background-process test" >&2
    exit 1
fi

DAEMON_RSS_KB=$(awk '/VmRSS:/ {print $2}' "/proc/$DAEMON_PID/status")
TASK_PID=$(head -n 1 "/sys/fs/cgroup/tinybox-$TASK_ID/cgroup.procs")
TASK_RSS_KB=$(awk '/VmRSS:/ {print $2}' "/proc/$TASK_PID/status")
"$TINYBOX" agent destroy "$TASK_ID"
DESTROY_CLEAN=$([ ! -e "/sys/fs/cgroup/tinybox-$TASK_ID" ] && \
    [ ! -e "/var/lib/tinybox/tasks/$TASK_ID" ] && echo true || echo false)
TASK_ID=

python3 - "$DEMO_ROOT/samples" "$ITERATIONS" "$DAEMON_RSS_KB" "$TASK_RSS_KB" \
    "$STATE_BYTES_BEFORE" "$STATE_BYTES_AFTER" "$DESTROY_CLEAN" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
def summary(name):
    values = sorted(float(x) for x in (root / f"{name}.ms").read_text().split())
    pick = lambda q: values[min(len(values)-1, round((len(values)-1)*q))]
    return {"p50_ms": pick(.50), "p95_ms": pick(.95)}
result = {
    "schema_version": 1,
    "iterations": int(sys.argv[2]),
    "user_steps": {"build": 1, "daemon_start": 1, "agent_run": 1},
    "latency": {name: summary(name) for name in ("bare", "cold", "warm")},
    "idle_rss_kb": {"daemon": int(sys.argv[3]), "task": int(sys.argv[4])},
    "environment_bytes": {
        "before_cache": int(sys.argv[5]), "after_cache": int(sys.argv[6]),
        "delta": int(sys.argv[6]) - int(sys.argv[5]),
    },
    "checks": {"build_test": True, "timeout": True, "background_reaped": True,
               "destroy_clean": sys.argv[7] == "true"},
}
print(json.dumps(result, sort_keys=True))
print("tinybox demo: build/test PASS; timeout PASS; background reap PASS; destroy clean PASS", file=sys.stderr)
PY
