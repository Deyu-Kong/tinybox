#!/bin/sh
set -eu

user_home=$(getent passwd "$(id -un)" | cut -d: -f6)
prefix=${TINYBOX_INSTALL_PREFIX:-$user_home/.local}

cargo build --release --locked
install -d "$prefix/bin" "$prefix/share/tinybox/adapters/opencode" \
    "$prefix/share/tinybox/adapters/pi" "$prefix/share/tinybox/adapters/shared"
install -m 0755 target/release/tinybox "$prefix/bin/tinybox"
install -m 0755 scripts/tinybox-opencode "$prefix/bin/tinybox-opencode"
install -m 0755 scripts/install_opencode_adapter.sh "$prefix/bin/tinybox-install-opencode-adapter"
install -m 0644 adapters/opencode/bash.ts "$prefix/share/tinybox/adapters/opencode/"
install -m 0644 adapters/pi/tinybox.ts "$prefix/share/tinybox/adapters/pi/"
install -m 0644 adapters/shared/runtime.js "$prefix/share/tinybox/adapters/shared/"
printf 'installed tinybox to %s/bin\n' "$prefix"
printf 'run: sudo %s/bin/tinybox doctor\n' "$prefix"
