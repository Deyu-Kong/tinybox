#!/bin/sh
set -eu

workspace=${1:-.}
source_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)/adapters/opencode
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
