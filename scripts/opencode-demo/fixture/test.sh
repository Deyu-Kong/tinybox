#!/bin/sh
set -eu
test "$(cat dist/result.txt)" = tinybox-build-ok
test "$(cat "$XDG_CACHE_HOME/demo-cache")" = dependency-ready
printf 'TEST=PASS artifact-and-cache-reused\n'
