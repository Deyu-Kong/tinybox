#!/bin/bash
set -euo pipefail

node --test adapters/opencode/runtime.test.mjs

fixture=$(mktemp -d /var/tmp/tinybox-m3.XXXXXX)
cleanup() {
    find "$fixture" -depth -delete 2>/dev/null || true
}
trap cleanup EXIT

scripts/install_opencode_adapter.sh "$fixture" >/dev/null
opencode_bin=$(command -v opencode 2>/dev/null || true)
if [ -z "$opencode_bin" ] && [ -n "${SUDO_USER:-}" ]; then
    user_home=$(getent passwd "$SUDO_USER" | cut -d: -f6)
    for candidate in "$user_home"/.nvm/versions/node/*/bin/opencode; do
        [ -x "$candidate" ] && opencode_bin=$candidate
    done
fi
if [ -n "$opencode_bin" ]; then
    (cd "$fixture" && "$opencode_bin" debug config >/dev/null)
    echo "PASS: OpenCode loads the tinybox bash adapter"
else
    echo "SKIP: opencode is not installed"
fi

if [ "$(id -u)" -ne 0 ]; then
    echo "SKIP: Agent runtime integration requires root"
    exit 77
fi

test_binary=
for candidate in target/debug/deps/agent_cli-*; do
    if [ -f "$candidate" ] && [ -x "$candidate" ]; then
        test_binary=$candidate
        break
    fi
done
if [ -z "$test_binary" ]; then
    echo "missing Agent CLI test binary; run cargo test --test agent_cli --no-run" >&2
    exit 1
fi
"$test_binary" --nocapture
