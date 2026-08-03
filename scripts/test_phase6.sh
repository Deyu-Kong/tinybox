#!/bin/bash
set -e

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"
BUNDLE=$(mktemp -d)
trap 'rm -rf "$BUNDLE"' EXIT
cat > "$BUNDLE/config.json" <<'EOF'
{"process":{"args":["sh","-c","test \"$TINYBOX_PHASE6\" = ok && echo hello-oci"],"env":["PATH=/usr/bin:/bin","TINYBOX_PHASE6=ok"]},"root":{"path":"/","readonly":true},"linux":{"namespaces":[{"type":"pid"},{"type":"mount"}]}}
EOF

OUTPUT=$($TINYBOX run --oci "$BUNDLE")
if [ "$OUTPUT" != "hello-oci" ]; then
    echo "FAIL: got: $OUTPUT"
    exit 1
fi
echo "Phase 6 OCI bundle test passed"
