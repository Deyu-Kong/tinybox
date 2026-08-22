#!/bin/bash
set -euo pipefail

fixture=$(mktemp -d /var/tmp/tinybox-m5.XXXXXX)
cleanup() {
    find "$fixture" -depth -delete 2>/dev/null || true
}
trap cleanup EXIT

TINYBOX_INSTALL_PREFIX="$fixture/prefix" ./scripts/install.sh
installed=$fixture/prefix/bin/tinybox
"$installed" doctor --json | python3 -c '
import json, sys
checks = json.load(sys.stdin)
assert checks and not any(item["status"] == "fail" for item in checks)
'

mkdir -p "$fixture/project"
HOME="$fixture/home" "$installed" agent integrate opencode --project "$fixture/project" >/dev/null
test -f "$fixture/project/.opencode/tools/bash.ts"
test -f "$fixture/project/.opencode/tools/runtime.js"

HOME="$fixture/home" "$installed" agent integrate opencode >/dev/null
HOME="$fixture/home" "$installed" agent integrate pi >/dev/null
test -f "$fixture/home/.config/opencode/tools/bash.ts"
test -f "$fixture/home/.config/opencode/tools/runtime.js"
test -f "$fixture/home/.pi/agent/extensions/tinybox/index.ts"
test -f "$fixture/home/.pi/agent/extensions/tinybox/runtime.js"

TINYBOX_INSTALL_PREFIX="$fixture/prefix" ./scripts/uninstall.sh >/dev/null
test ! -e "$installed"
echo "PASS: install, doctor, adapter assets, and uninstall"

echo "INFO: runtime acceptance is separate: sudo ./scripts/demo_local_agent.sh 3"
