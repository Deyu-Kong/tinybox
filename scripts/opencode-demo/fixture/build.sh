#!/bin/sh
set -eu
mkdir -p dist
chmod 0777 dist
printf 'tinybox-build-ok\n' > dist/result.txt
printf 'BUILD=PASS cwd=%s\n' "$PWD"
