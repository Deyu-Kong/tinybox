#!/bin/sh
set -eu

workspace=${1:-.}
install_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -d "$install_root/adapters/opencode" ]; then
    source_dir=$install_root/adapters/opencode
else
    source_dir=$install_root/share/tinybox/adapters/opencode
fi
target_dir=$workspace/.opencode/tools

mkdir -p "$target_dir"
for name in bash.ts runtime.js; do
    source_file=$source_dir/$name
    target_file=$target_dir/$name
    if [ -e "$target_file" ] && ! cmp -s "$source_file" "$target_file"; then
        echo "refusing to overwrite existing $target_file" >&2
        exit 1
    fi
    cp "$source_file" "$target_file"
done
echo "$target_dir/bash.ts"
