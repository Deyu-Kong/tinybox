#!/bin/bash
set -euo pipefail

node --test adapters/opencode/runtime.test.mjs

fixture=$(mktemp -d /var/tmp/tinybox-m3.XXXXXX)
cleanup() {
    find "$fixture" -depth -delete 2>/dev/null || true
}
trap cleanup EXIT

HOME="$fixture/home" XDG_CONFIG_HOME="$fixture/config" \
    target/debug/tinybox agent integrate opencode >/dev/null
HOME="$fixture/home" XDG_CONFIG_HOME="$fixture/config" \
    target/debug/tinybox agent integrate pi >/dev/null
test -f "$fixture/config/opencode/tools/bash.ts"
test -f "$fixture/config/opencode/tools/runtime.js"
test -f "$fixture/home/.pi/agent/extensions/tinybox/index.ts"
test -f "$fixture/home/.pi/agent/extensions/tinybox/runtime.js"
mkdir -p "$fixture/project"
opencode_bin=$(command -v opencode 2>/dev/null || true)
if [ -z "$opencode_bin" ] && [ -n "${SUDO_USER:-}" ]; then
    user_home=$(getent passwd "$SUDO_USER" | cut -d: -f6)
    for candidate in "$user_home"/.nvm/versions/node/*/bin/opencode; do
        [ -x "$candidate" ] && opencode_bin=$candidate
    done
fi
if [ -n "$opencode_bin" ]; then
    (cd "$fixture/project" && HOME="$fixture/home" XDG_CONFIG_HOME="$fixture/config" \
        OPENCODE_CONFIG_DIR="$fixture/config/opencode" "$opencode_bin" debug config >/dev/null)
    echo "PASS: OpenCode loads the user-level tinybox bash adapter"
else
    echo "SKIP: opencode is not installed"
fi

pi_bin=$(command -v pi 2>/dev/null || true)
if [ -z "$pi_bin" ] && [ -n "${SUDO_USER:-}" ]; then
    user_home=$(getent passwd "$SUDO_USER" | cut -d: -f6)
    for candidate in "$user_home"/.nvm/versions/node/*/bin/pi; do
        [ -x "$candidate" ] && pi_bin=$candidate
    done
fi
if [ -n "$pi_bin" ]; then
    HOME="$fixture/home" "$pi_bin" --extension \
        "$fixture/home/.pi/agent/extensions/tinybox/index.ts" --version >/dev/null
    echo "PASS: Pi loads the user-level tinybox bash adapter"
else
    echo "SKIP: Pi is not installed; install and source-contract tests passed"
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
