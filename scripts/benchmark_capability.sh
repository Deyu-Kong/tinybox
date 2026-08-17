#!/bin/bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "benchmark requires root" >&2
    exit 77
fi

ITERATIONS=${1:-10}
TINYBOX=${TINYBOX:-./target/release/tinybox}
BENCH_TMP=$(mktemp -d)
BENCH_DAEMON_PID=""
cleanup() {
    [ -z "$BENCH_DAEMON_PID" ] || kill "$BENCH_DAEMON_PID" 2>/dev/null || true
    rm -rf "$BENCH_TMP"
}
trap cleanup EXIT
cat >"$BENCH_TMP/policy.json" <<'EOF'
{"version":1,"filesystem":[],"network":[],"resources":{"memory_bytes":268435456,"cpus":1.0,"pids":50},"phases":[]}
EOF

measure() {
    local name=$1
    shift
    : >"$BENCH_TMP/$name.ms"
    : >"$BENCH_TMP/$name.rss"
    for _ in $(seq 1 "$ITERATIONS"); do
        /usr/bin/time -f '%e %M' -o "$BENCH_TMP/sample" "$@"
        awk '{printf "%.3f\n", $1 * 1000}' "$BENCH_TMP/sample" >>"$BENCH_TMP/$name.ms"
        awk '{print $2}' "$BENCH_TMP/sample" >>"$BENCH_TMP/$name.rss"
    done
}

measure native /bin/true
measure sandbox "$TINYBOX" run --root / --policy "$BENCH_TMP/policy.json" -- /bin/true
if command -v runc >/dev/null && [ -x /usr/bin/busybox ]; then
    mkdir -p "$BENCH_TMP/runc/rootfs/bin"
    cp /usr/bin/busybox "$BENCH_TMP/runc/rootfs/bin/busybox"
    cat >"$BENCH_TMP/runc/config.json" <<'EOF'
{"ociVersion":"1.0.2","process":{"terminal":false,"user":{"uid":0,"gid":0},"args":["/bin/busybox","true"],"env":["PATH=/bin"],"cwd":"/","capabilities":{"bounding":[],"effective":[],"inheritable":[],"permitted":[],"ambient":[]},"noNewPrivileges":true},"root":{"path":"rootfs","readonly":true},"hostname":"runc-bench","mounts":[{"destination":"/proc","type":"proc","source":"proc"}],"linux":{"namespaces":[{"type":"pid"},{"type":"mount"},{"type":"uts"},{"type":"ipc"},{"type":"network"}]}}
EOF
    cat >"$BENCH_TMP/runc-run" <<EOF
#!/bin/sh
exec runc run --bundle "$BENCH_TMP/runc" "tinybox-bench-\$\$"
EOF
    chmod +x "$BENCH_TMP/runc-run"
    measure runc "$BENCH_TMP/runc-run"
fi
"$TINYBOX" daemon --listen 127.0.0.1:18104 >"$BENCH_TMP/daemon.log" 2>&1 &
BENCH_DAEMON_PID=$!
for _ in $(seq 1 30); do
    curl -fsS http://127.0.0.1:18104/metrics >/dev/null 2>&1 && break
    sleep 0.1
done
: >"$BENCH_TMP/phase.ms"
: >"$BENCH_TMP/audit.ms"
for _ in $(seq 1 "$ITERATIONS"); do
    REQUEST='{"rootfs":"/","command":["/bin/sleep","30"],"policy":{"version":1,"filesystem":[],"network":[],"resources":{"memory_bytes":268435456,"cpus":1.0,"pids":50},"phases":[{"name":"install","network":[],"resources":{"memory_bytes":268435456,"cpus":1.0,"pids":50},"next":["build"]},{"name":"build","network":[],"resources":{"memory_bytes":134217728,"cpus":0.5,"pids":20},"next":[]}]}}'
    CREATE=$(curl -fsS -X POST http://127.0.0.1:18104/api/sandboxes \
        -H 'Content-Type: application/json' -d "$REQUEST")
    ID=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<<"$CREATE")
    for _ in $(seq 1 30); do
        [ -e "/sys/fs/cgroup/tinybox-$ID/cgroup.procs" ] && break
        sleep 0.01
    done
    curl -fsS -o /dev/null -w '%{time_total}\n' -X POST \
        "http://127.0.0.1:18104/api/sandboxes/$ID/phase" \
        -H 'Content-Type: application/json' \
        -d '{"phase":"build","expected_generation":0}' | \
        awk '{printf "%.3f\n", $1 * 1000}' >>"$BENCH_TMP/phase.ms"
    curl -fsS -o /dev/null -w '%{time_total}\n' \
        "http://127.0.0.1:18104/api/sandboxes/$ID/audit" | \
        awk '{printf "%.3f\n", $1 * 1000}' >>"$BENCH_TMP/audit.ms"
    curl -fsS -X DELETE "http://127.0.0.1:18104/api/sandboxes/$ID" >/dev/null
done

python3 - "$BENCH_TMP" "$ITERATIONS" <<'PY'
import json, pathlib, statistics, sys
root = pathlib.Path(sys.argv[1])
result = {"iterations": int(sys.argv[2])}
for name in ("native", "sandbox", "runc"):
    if not (root / f"{name}.ms").exists():
        continue
    latency = sorted(float(x) for x in (root / f"{name}.ms").read_text().split())
    rss = [int(x) for x in (root / f"{name}.rss").read_text().split()]
    pick = lambda q: latency[min(len(latency)-1, int((len(latency)-1)*q))]
    result[name] = {"latency_ms_p50": pick(.50), "latency_ms_p95": pick(.95), "max_rss_kb": max(rss)}
result["sandbox_overhead_ms_p50"] = result["sandbox"]["latency_ms_p50"] - result["native"]["latency_ms_p50"]
for name in ("phase", "audit"):
    values = sorted(float(x) for x in (root / f"{name}.ms").read_text().split())
    pick = lambda q: values[min(len(values)-1, int((len(values)-1)*q))]
    result[f"{name}_api"] = {"latency_ms_p50": pick(.50), "latency_ms_p95": pick(.95)}
print(json.dumps(result, sort_keys=True))
PY
