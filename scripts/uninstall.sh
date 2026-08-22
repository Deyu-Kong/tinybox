#!/bin/sh
set -eu

user_home=$(getent passwd "$(id -un)" | cut -d: -f6)
prefix=${TINYBOX_INSTALL_PREFIX:-$user_home/.local}

for path in \
    "$prefix/bin/tinybox" \
    "$prefix/bin/tinybox-opencode" \
    "$prefix/bin/tinybox-install-opencode-adapter" \
    "$prefix/share/tinybox/adapters/opencode/bash.ts" \
    "$prefix/share/tinybox/adapters/opencode/runtime.js" \
    "$prefix/share/tinybox/adapters/pi/tinybox.ts"
do
    [ ! -e "$path" ] || unlink "$path"
done
echo "removed tinybox files from $prefix; empty parent directories were retained"
