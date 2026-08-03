#!/bin/bash
set -e

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: This script must be run as root"
    exit 1
fi

TINYBOX="./target/debug/tinybox"
STORE="/var/lib/tinybox/images"
TMP_TAR=$(mktemp /tmp/tinybox-fixture.XXXXXX.tar)
TINYBOX_ALIAS="alpine-test-$RANDOM"
trap 'rm -f "$TMP_TAR"; rm -rf "$STORE/$TINYBOX_ALIAS"' EXIT

echo -n "Preparing fixture rootfs tar... "
ROOTFS=$(mktemp -d)
mkdir -p "$ROOTFS/bin" "$ROOTFS/usr/bin" "$ROOTFS/proc" "$ROOTFS/tmp"
cp /bin/sh "$ROOTFS/bin/sh"
cp /bin/echo "$ROOTFS/bin/echo"
cp /bin/cat "$ROOTFS/bin/cat"
# Copy required shared libraries
for bin in sh echo cat; do
    ldd /bin/$bin 2>/dev/null | grep -o '/[^ ]*' | while read lib; do
        dir=$(dirname "$lib")
        mkdir -p "$ROOTFS$dir"
        cp -n "$lib" "$ROOTFS$lib" 2>/dev/null || true
    done
done
ldconfig -r "$ROOTFS" 2>/dev/null || true
tar -cf "$TMP_TAR" -C "$ROOTFS" .
rm -rf "$ROOTFS"
echo "OK"

echo -n "Importing image via tinybox image import... "
$TINYBOX image import "$TMP_TAR" --alias "$TINYBOX_ALIAS" >/dev/null
echo "OK"

echo -n "Listing imported image... "
if ! $TINYBOX image list | grep -q "$TINYBOX_ALIAS"; then
    echo "FAIL"
    exit 1
fi
echo "OK"

echo -n "Running sandbox with --image... "
OUTPUT=$($TINYBOX run --image "$TINYBOX_ALIAS" -- /bin/sh -c 'echo hello-image')
if [ "$OUTPUT" = "hello-image" ]; then
    echo "PASS"
else
    echo "FAIL (got: $OUTPUT)"
    exit 1
fi

echo -n "Removing image... "
$TINYBOX image remove "$TINYBOX_ALIAS" >/dev/null
if $TINYBOX image list | grep -q "$TINYBOX_ALIAS"; then
    echo "FAIL"
    exit 1
fi
echo "OK"

echo "=== Phase 9 image tests passed ==="
