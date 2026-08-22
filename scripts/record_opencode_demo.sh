#!/bin/bash
set -euo pipefail

TINYBOX=${TINYBOX:-./target/release/tinybox}
DAEMON=${TINYBOX_DAEMON:-127.0.0.1:18140}
REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
DEMO_ROOT=
TASK_ID=

if [ ! -x "$TINYBOX" ]; then
    printf 'missing tinybox binary: %s\n' "$TINYBOX" >&2
    exit 1
fi
TINYBOX=$(readlink -f "$TINYBOX")

cleanup() {
    if [ -n "$TASK_ID" ]; then
        "$TINYBOX" agent destroy "$TASK_ID" >/dev/null 2>&1 || true
    fi
    if [ -n "$DEMO_ROOT" ] && [ -d "$DEMO_ROOT" ]; then
        find "$DEMO_ROOT" -depth -delete 2>/dev/null || true
    fi
}
trap cleanup EXIT HUP INT TERM

DEMO_ROOT=$(mktemp -d /var/tmp/tinybox-opencode-recording.XXXXXX)
WORKSPACE=$DEMO_ROOT/workspace
SECRET_PATH=$DEMO_ROOT/outside/host-secret-canary.txt
mkdir -p "$WORKSPACE" "$DEMO_ROOT/outside"
cp "$REPO_ROOT"/scripts/opencode-demo/fixture/{README.md,build.sh,test.sh} "$WORKSPACE/"
cp "$REPO_ROOT/scripts/opencode-demo/fixture/host-secret-canary.txt" "$SECRET_PATH"
"$REPO_ROOT/scripts/install_opencode_adapter.sh" "$WORKSPACE" >/dev/null

clear
printf '\033[1;36m tinybox + OpenCode\033[0m\n'
printf ' Persistent environment + workspace isolation for local coding agents\n\n'
sleep 2

printf '\033[1;32m$\033[0m tinybox agent run demo-project --detach\n'
TASK_ID=$("$TINYBOX" agent run "$WORKSPACE" --daemon "$DAEMON" --detach)
printf 'task created: \033[33m%s\033[0m\n\n' "$TASK_ID"
sleep 2

printf '\033[1;32m$\033[0m opencode run "Build, reuse state, test, and check isolation"\n\n'
cd "$WORKSPACE"
TINYBOX_TASK_ID="$TASK_ID" TINYBOX_BIN="$TINYBOX" \
    opencode run --auto \
    "Use bash in exactly four separate tool calls. First run ./build.sh. Second write dependency-ready to \$XDG_CACHE_HOME/demo-cache. Third run ./test.sh and then print the cache file. Fourth try to read $SECRET_PATH; if reading fails, print exactly SECURITY_CHECK=HOST_SECRET_BLOCKED. Do not use any other tools. Finally summarize build, persistent cache, tests, and isolation."

printf '\n\033[1;32m✓ four OpenCode bash calls reused persistent task %s\033[0m\n' "$TASK_ID"
printf '\033[1;32m$\033[0m tinybox agent destroy %s\n' "$TASK_ID"
"$TINYBOX" agent destroy "$TASK_ID" >/dev/null
TASK_ID=
printf '\033[1;32m✓ task destroyed; sandbox state and processes reclaimed\033[0m\n'
sleep 5
