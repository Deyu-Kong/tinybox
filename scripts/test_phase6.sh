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
echo "Test 1: OCI bundle executes with env... PASS"

# P1-1: an OCI config requesting only {pid,mount} must NOT create a private
# netns (shares host), while adding {net} must create one. Compare the netns
# inode readlink inside the sandbox vs the host's.
HOST_NET=$(readlink /proc/self/ns/net)

cat > "$BUNDLE/config.json" <<'EOF'
{"process":{"args":["sh","-c","readlink /proc/self/ns/net"]},"root":{"path":"/"},"linux":{"namespaces":[{"type":"pid"},{"type":"mount"}]}}
EOF
echo -n "Test 2: namespace subset {pid,mount} shares host netns (P1-1)... "
OUT=$($TINYBOX run --oci "$BUNDLE")
if [ "$OUT" = "$HOST_NET" ]; then
    echo "PASS"
else
    echo "FAIL (expected host netns $HOST_NET, got $OUT)"
    exit 1
fi

cat > "$BUNDLE/config.json" <<'EOF'
{"process":{"args":["sh","-c","readlink /proc/self/ns/net"]},"root":{"path":"/"},"linux":{"namespaces":[{"type":"pid"},{"type":"mount"},{"type":"net"}]}}
EOF
echo -n "Test 3: namespace subset with {net} creates a private netns (P1-1)... "
OUT=$($TINYBOX run --oci "$BUNDLE")
if [ "$OUT" != "$HOST_NET" ]; then
    echo "PASS"
else
    echo "FAIL (expected a private netns, got host $OUT)"
    exit 1
fi

echo "=== Phase 6 tests passed ==="
